// mos6502 — a cycle-counting MOS 6502 core.
//
// What it does
//   The whole documented instruction set of the NMOS 6502: all fifty-six
//   official mnemonics in every addressing mode they are defined for,
//   which is 151 of the 256 opcodes. ADC, AND, ASL, BCC, BCS, BEQ, BIT,
//   BMI, BNE, BPL, BRK, BVC, BVS, CLC, CLD, CLI, CLV, CMP, CPX, CPY,
//   DEC, DEX, DEY, EOR, INC, INX, INY, JMP, JSR, LDA, LDX, LDY, LSR,
//   NOP, ORA, PHA, PHP, PLA, PLP, ROL, ROR, RTI, RTS, SBC, SEC, SED,
//   SEI, STA, STX, STY, TAX, TAY, TSX, TXA, TXS, TYA.
//
//   The thirteen addressing modes, with the cycles each takes. The
//   count is the number of bus cycles the core spends, which for a
//   memory that is always ready is the number of clocks:
//
//     implied / accumulator            2
//     immediate                        2
//     zero page                read 3, read-modify-write 5, write 3
//     zero page,X / zero page,Y  read 4, read-modify-write 6, write 4
//     absolute                 read 4, read-modify-write 6, write 4
//     absolute,X / absolute,Y  read 4 (+1 across a page), RMW 7, write 5
//     indexed indirect (zp,X)  read 6,                          write 6
//     indirect indexed (zp),Y  read 5 (+1 across a page),       write 6
//     indirect (JMP only)              5
//     relative (branches)      2, +1 if taken, +1 more across a page
//
//   and the instructions with a shape of their own: JMP absolute 3,
//   JSR 6, RTS 6, RTI 6, PHA / PHP 3, PLA / PLP 4, BRK 7, and an
//   interrupt or the reset sequence 7.
//
//   Like the original, the core performs **exactly one bus cycle per
//   clock**, so a cycle is a memory access and the counts above are
//   also the number of accesses. The addresses are the real ones too:
//   the discarded reads of an indexed mode, of a read-modify-write's
//   first write-back, and of the stack during JSR are all there, at the
//   addresses the original puts on the pins.
//
//   The processor status register. N, V, D, I, Z and C are real flip
//   flops; bit 5 reads as one and B is not a register at all but a
//   property of how the byte reached the stack, which this core
//   reproduces: PHP and BRK push with bit 4 set, an IRQ, an NMI and the
//   reset sequence push with it clear, and PLP and RTI ignore bits 4
//   and 5 of what they pull. V comes from ADC and SBC as the signed
//   overflow of the byte addition — set when the operands share a sign
//   and the result does not — and from BIT as bit 6 of the addressed
//   byte, with N as bit 7 and Z from `A & M`.
//
//   Decimal mode, when DECIMAL_MODE is 1. ADC and SBC with D set work
//   on packed binary-coded decimal the way the NMOS part does, which
//   includes the parts of it that look like bugs and are not:
//
//     ADC  the accumulator and C are the decimal result and its carry,
//          but Z is the *binary* sum's, and N and V come from the
//          intermediate after the low nibble is corrected and before
//          the high one is — so SED / SEC / LDA #$99 / ADC #$00 leaves
//          A = $00 with C = 1 and Z = 0.
//     SBC  the accumulator is the decimal difference, and N, V, Z and C
//          are all exactly the binary subtraction's.
//
//   With DECIMAL_MODE = 0 that arithmetic is not built. D is still a
//   flag — SED, CLD, PHP and PLP all see it — but ADC and SBC ignore
//   it and always work in binary. The footprint table in
//   `docs/ip-library.md` has both, which is the point of the
//   parameter: decimal mode is two more adders and a pair of
//   comparators, and a design that does not want them should not pay.
//
//   Interrupts, in the priority RES > NMI > IRQ > BRK:
//
//     RES  `rst_n` low resets the registers; releasing it runs the
//          seven-cycle sequence, which reads rather than writes the
//          three stack slots, leaves S three lower, sets I and jumps
//          through the vector at $FFFC.
//     NMI  `nmi` is edge triggered: a low-to-high transition sets a
//          latch that nothing but taking the interrupt clears, so a
//          pulse narrower than an instruction is not lost. Not masked
//          by I. Pushes PC and P with bit 4 clear, sets I, vectors
//          through $FFFA.
//     IRQ  `irq` is level triggered and masked by I: it is taken
//          between instructions whenever it is high and I is clear,
//          and it stays pending — and is taken again — while the line
//          is still high. Vectors through $FFFE.
//     BRK  a one-byte opcode with a padding byte after it: it pushes
//          the address *past* the padding byte, pushes P with bit 4
//          set, sets I and vectors through $FFFE, the same vector the
//          IRQ uses, so a handler tells them apart by the pushed bit 4.
//
//   `irq` and `nmi` are active high here; the original chip's pins are
//   active low, so a board inverts them. Both are sampled on `clk` and
//   must already be synchronous to it — put `cdc_sync` in front of one
//   that is not.
//
//   The documented quirks a program can see, each with a test of its
//   own in `tests/ip_library.rs`:
//
//     * the indirect JMP page-boundary bug. `JMP ($10FF)` reads the low
//       byte of its target from $10FF and the high byte from $1000, not
//       $1100: the pointer's low byte is incremented without a carry
//       into the high one.
//       (`mos6502_reproduces_the_indirect_jmp_page_bug`)
//     * the extra cycle when an indexed *read* crosses a page. LDA
//       $12FF,X with X = 1 reads $1200 first, discards it, and reads
//       $1300 in a fifth cycle. An indexed *write* or read-modify-write
//       always spends that cycle, crossing or not, which is why STA
//       $1234,X is five cycles and LDA $1234,X is four.
//       (`mos6502_spends_an_extra_cycle_when_an_indexed_read_crosses_a_page`)
//     * branch timing. A branch not taken is two cycles, taken is
//       three, and taken onto another page is four.
//       (`mos6502_times_branches_by_whether_they_are_taken_and_cross`)
//     * a read-modify-write writes twice. INC, DEC and the four shifts
//       read their address, write back the byte they read, and then
//       write the result — three accesses to one address, which a
//       memory-mapped register can see.
//       (`mos6502_writes_a_read_modify_write_byte_back_before_the_result`)
//     * the stack wraps inside page one. S is eight bits and the
//       address is always $0100 + S, so pushing past $0100 lands on
//       $01FF and pulling past $01FF lands on $0100; nothing outside
//       page one is ever touched.
//       (`mos6502_wraps_the_stack_inside_page_one`)
//     * zero-page indexing wraps inside page zero. LDA $FF,X with
//       X = 2 reads $0001, never $0101, and the pointer of (zp,X) and
//       (zp),Y is fetched from two addresses that both wrap.
//       (`mos6502_wraps_zero_page_indexing_and_its_pointers`)
//     * the one-instruction delay of CLI, SEI and PLP. The interrupt
//       decision for an instruction is made with the flags as they were
//       *before* it, so an IRQ pending when SEI runs is still taken
//       after it, and one pending when CLI runs is not taken until the
//       instruction after that.
//       (`mos6502_delays_the_effect_of_cli_and_sei_by_one_instruction`)
//
//   The bus. One access per cycle: `addr` and `we` are valid for the
//   whole cycle, a write presents `dout`, a read takes `din` at the
//   rising edge. `ready` low stalls the core with the access held, so
//   slow memory costs whole cycles and nothing else; `sync` is high
//   during an opcode fetch, the same signal the original brings out on
//   its SYNC pin. The three `dbg_*` outputs exist for a testbench and
//   can be left unconnected.
//
// What it does not do
//   **The cycle *count* is the original's; the cycle *shape* is not.**
//   This is a synchronous core on one clock edge, not a two-phase part:
//   there is no φ1 / φ2, no address setup before the clock, and no
//   half-cycle data window. A cycle here is one clock, an access is
//   valid for all of it, and `ready` stalls reads and writes alike —
//   where the original's RDY only stops it on a read and lets a write
//   through. So a program's cycle count matches the tables exactly, and
//   an oscilloscope would not.
//
//   The 105 undocumented opcodes are **out of scope and defined
//   anyway**: every one of them behaves exactly like NOP — two cycles,
//   one discarded read of the byte after the opcode, no register and no
//   flag touched. That is deterministic and documented, but it is not
//   what the NMOS part does, so a program that uses LAX or SLO will run
//   off the rails here. It will do so the same way every time.
//
//   No interrupt hijacking: an NMI arriving during a BRK does not
//   redirect it to the NMI vector, as it does on the real part. No
//   interrupt polled inside a taken branch, so the "branch delays an
//   IRQ" corner is not reproduced. Interrupts are decided once, at the
//   end of each instruction; nothing is sampled mid-instruction.
//
//   No 65C02 and no 65816: none of the added opcodes, none of the
//   added addressing modes, and none of the fixes — the indirect JMP
//   bug is *kept*, and decimal mode does not clear V or cost the extra
//   cycle the CMOS part spends on it.
//
//   RES leaves N, V, Z, C and D undefined on the real part; here they
//   are cleared, because a hardware register has to start somewhere and
//   "undefined" is not a thing this core is allowed to be. I is set by
//   the reset sequence, as it is on the original, and S ends at $FD.
//
//   There is no SO pin, no RDY-during-write, no bus-error input and no
//   halt: a memory that cannot answer must simply hold `ready` low, and
//   the core waits for ever. `dout` is driven in every cycle rather
//   than floating on a read, because this is a pair of unidirectional
//   buses and not the original's bidirectional one.
module mos6502 #(
    // 1 builds the packed binary-coded decimal arithmetic ADC and SBC
    // use when the D flag is set; 0 leaves D a flag nothing reads.
    parameter DECIMAL_MODE = 1
) (
    input  wire        clk,
    input  wire        rst_n,

    // The bus. One access per cycle, held until `ready` is seen high at
    // a rising edge.
    output wire [15:0] addr,
    output wire [7:0]  dout,
    input  wire [7:0]  din,
    output wire        we,
    input  wire        ready,
    // High for the cycle that fetches an opcode.
    output wire        sync,

    // Interrupt requests, active high and synchronous to `clk`.
    input  wire        irq,
    input  wire        nmi,

    // Retirement trace. `dbg_pc` is the address of the opcode whose
    // instruction finished on the previous edge.
    output wire [15:0] dbg_pc,
    output wire        dbg_retire,
    output wire        dbg_trap
);
    // -----------------------------------------------------------------
    // Addressing modes.
    // -----------------------------------------------------------------
    localparam [3:0] AM_IMP = 4'd0;   // implied
    localparam [3:0] AM_ACC = 4'd1;   // accumulator
    localparam [3:0] AM_IMM = 4'd2;   // #$nn
    localparam [3:0] AM_ZP  = 4'd3;   // $nn
    localparam [3:0] AM_ZPX = 4'd4;   // $nn,X
    localparam [3:0] AM_ZPY = 4'd5;   // $nn,Y
    localparam [3:0] AM_ABS = 4'd6;   // $nnnn
    localparam [3:0] AM_ABX = 4'd7;   // $nnnn,X
    localparam [3:0] AM_ABY = 4'd8;   // $nnnn,Y
    localparam [3:0] AM_IND = 4'd9;   // ($nnnn), JMP only
    localparam [3:0] AM_IZX = 4'd10;  // ($nn,X)
    localparam [3:0] AM_IZY = 4'd11;  // ($nn),Y
    localparam [3:0] AM_REL = 4'd12;  // branch displacement

    // -----------------------------------------------------------------
    // Operations.
    // -----------------------------------------------------------------
    localparam [5:0] OP_NOP = 6'd0;
    localparam [5:0] OP_ORA = 6'd1;
    localparam [5:0] OP_AND = 6'd2;
    localparam [5:0] OP_EOR = 6'd3;
    localparam [5:0] OP_ADC = 6'd4;
    localparam [5:0] OP_SBC = 6'd5;
    localparam [5:0] OP_CMP = 6'd6;
    localparam [5:0] OP_CPX = 6'd7;
    localparam [5:0] OP_CPY = 6'd8;
    localparam [5:0] OP_BIT = 6'd9;
    localparam [5:0] OP_LDA = 6'd10;
    localparam [5:0] OP_LDX = 6'd11;
    localparam [5:0] OP_LDY = 6'd12;
    localparam [5:0] OP_STA = 6'd13;
    localparam [5:0] OP_STX = 6'd14;
    localparam [5:0] OP_STY = 6'd15;
    localparam [5:0] OP_ASL = 6'd16;
    localparam [5:0] OP_LSR = 6'd17;
    localparam [5:0] OP_ROL = 6'd18;
    localparam [5:0] OP_ROR = 6'd19;
    localparam [5:0] OP_INC = 6'd20;
    localparam [5:0] OP_DEC = 6'd21;
    localparam [5:0] OP_TAX = 6'd22;
    localparam [5:0] OP_TAY = 6'd23;
    localparam [5:0] OP_TXA = 6'd24;
    localparam [5:0] OP_TYA = 6'd25;
    localparam [5:0] OP_TSX = 6'd26;
    localparam [5:0] OP_TXS = 6'd27;
    localparam [5:0] OP_INX = 6'd28;
    localparam [5:0] OP_INY = 6'd29;
    localparam [5:0] OP_DEX = 6'd30;
    localparam [5:0] OP_DEY = 6'd31;
    localparam [5:0] OP_CLC = 6'd32;
    localparam [5:0] OP_SEC = 6'd33;
    localparam [5:0] OP_CLI = 6'd34;
    localparam [5:0] OP_SEI = 6'd35;
    localparam [5:0] OP_CLV = 6'd36;
    localparam [5:0] OP_CLD = 6'd37;
    localparam [5:0] OP_SED = 6'd38;
    localparam [5:0] OP_JMP = 6'd39;
    localparam [5:0] OP_JSR = 6'd40;
    localparam [5:0] OP_RTS = 6'd41;
    localparam [5:0] OP_RTI = 6'd42;
    localparam [5:0] OP_BRK = 6'd43;
    localparam [5:0] OP_PHA = 6'd44;
    localparam [5:0] OP_PHP = 6'd45;
    localparam [5:0] OP_PLA = 6'd46;
    localparam [5:0] OP_PLP = 6'd47;
    localparam [5:0] OP_BRA = 6'd48;  // the eight conditional branches

    // What the addressing mode has to do with memory once the effective
    // address is known.
    localparam [1:0] K_NONE  = 2'd0;  // no operand in memory
    localparam [1:0] K_READ  = 2'd1;  // read it
    localparam [1:0] K_WRITE = 2'd2;  // write it
    localparam [1:0] K_RMW   = 2'd3;  // read it, write it back, write the result

    // Which sequence put the core into the interrupt states.
    localparam [1:0] K_BRK = 2'd0;
    localparam [1:0] K_IRQ = 2'd1;
    localparam [1:0] K_NMI = 2'd2;
    localparam [1:0] K_RES = 2'd3;

    // -----------------------------------------------------------------
    // States. One state is one bus cycle.
    // -----------------------------------------------------------------
    localparam [5:0] S_FETCH  = 6'd0;   // opcode at PC, PC++
    localparam [5:0] S_IMPL   = 6'd1;   // discarded read at PC; implied execute
    localparam [5:0] S_IMM    = 6'd2;   // operand at PC, PC++
    localparam [5:0] S_REL    = 6'd3;   // branch displacement at PC, PC++
    localparam [5:0] S_ZP     = 6'd4;   // zero-page address at PC, PC++
    localparam [5:0] S_ZPIDX  = 6'd5;   // discarded read at $00nn; add the index
    localparam [5:0] S_ABSL   = 6'd6;   // address low at PC, PC++
    localparam [5:0] S_ABSH   = 6'd7;   // address high at PC, PC++
    localparam [5:0] S_INDL   = 6'd8;   // pointer low at $00nn
    localparam [5:0] S_INDH   = 6'd9;   // pointer high at $00nn+1, wrapped
    localparam [5:0] S_JMPL   = 6'd10;  // JMP (): target low
    localparam [5:0] S_JMPH   = 6'd11;  // JMP (): target high, low byte wrapped
    localparam [5:0] S_READ   = 6'd12;  // operand at the effective address
    localparam [5:0] S_WRFIX  = 6'd13;  // the always-spent cycle of an indexed write
    localparam [5:0] S_WRITE  = 6'd14;  // store
    localparam [5:0] S_RMWR   = 6'd15;  // read-modify-write: read
    localparam [5:0] S_RMWW1  = 6'd16;  // read-modify-write: write the byte back
    localparam [5:0] S_RMWW2  = 6'd17;  // read-modify-write: write the result
    localparam [5:0] S_BR1    = 6'd18;  // branch taken: add the displacement
    localparam [5:0] S_BR2    = 6'd19;  // branch crossed a page: carry into PCH
    localparam [5:0] S_STKD   = 6'd20;  // stack instruction: discarded read at PC
    localparam [5:0] S_PUSH   = 6'd21;  // PHA / PHP
    localparam [5:0] S_PULLD  = 6'd22;  // discarded read at $0100+S, S++
    localparam [5:0] S_PULL   = 6'd23;  // PLA / PLP
    localparam [5:0] S_RTIP   = 6'd24;  // RTI: pull P, S++
    localparam [5:0] S_RTSPL  = 6'd25;  // pull PCL, S++
    localparam [5:0] S_RTSPH  = 6'd26;  // pull PCH
    localparam [5:0] S_RTSPC  = 6'd27;  // RTS: discarded read at PC, PC++
    localparam [5:0] S_JSRD   = 6'd28;  // JSR: discarded read at $0100+S
    localparam [5:0] S_JSRPH  = 6'd29;  // JSR: push PCH
    localparam [5:0] S_JSRPL  = 6'd30;  // JSR: push PCL
    localparam [5:0] S_JSRH   = 6'd31;  // JSR: target high at PC
    localparam [5:0] S_BRKPC  = 6'd32;  // BRK: the padding byte at PC, PC++
    localparam [5:0] S_INTD   = 6'd33;  // interrupt: discarded read at PC
    localparam [5:0] S_INTD2  = 6'd34;  // interrupt: discarded read at PC
    localparam [5:0] S_INTPH  = 6'd35;  // push PCH (a read during reset)
    localparam [5:0] S_INTPL  = 6'd36;  // push PCL (a read during reset)
    localparam [5:0] S_INTP   = 6'd37;  // push P   (a read during reset), set I
    localparam [5:0] S_VECL   = 6'd38;  // vector low
    localparam [5:0] S_VECH   = 6'd39;  // vector high

    // -----------------------------------------------------------------
    // Architectural and internal state.
    // -----------------------------------------------------------------
    reg [7:0]  a_r;
    reg [7:0]  x_r;
    reg [7:0]  y_r;
    reg [7:0]  s_r;
    reg        p_n;
    reg        p_v;
    reg        p_d;
    reg        p_i;
    reg        p_z;
    reg        p_c;
    reg [15:0] pc;

    reg [7:0]  ir;        // the opcode being executed
    reg [7:0]  md;        // the byte a cycle read and a later one needs
    reg [7:0]  bl;        // a zero-page address or pointer base
    reg [7:0]  adl;       // effective address, low
    reg [7:0]  adh;       // effective address, high
    reg        fix;       // an indexed address whose high byte is one short
    reg [5:0]  st;
    reg [1:0]  int_kind;
    reg        nmi_q;     // `nmi` one cycle ago, for the edge
    reg        nmi_pend;  // an edge seen and not yet serviced
    reg [15:0] op_pc;     // where the opcode being executed came from

    // -----------------------------------------------------------------
    // Decode. During a fetch the word on the bus is the opcode; every
    // other cycle it is the one in `ir`, so one decoder serves both and
    // the fetch cycle can already pick the next state.
    // -----------------------------------------------------------------
    wire [7:0] opc = (st == S_FETCH) ? din : ir;

    reg [3:0] am;
    reg [5:0] op;
    always @(*) begin
        // Every opcode not named below is undocumented, and is NOP.
        am = AM_IMP;
        op = OP_NOP;
        case (opc)
            // ADC
            8'h69: begin op = OP_ADC; am = AM_IMM; end
            8'h65: begin op = OP_ADC; am = AM_ZP;  end
            8'h75: begin op = OP_ADC; am = AM_ZPX; end
            8'h6D: begin op = OP_ADC; am = AM_ABS; end
            8'h7D: begin op = OP_ADC; am = AM_ABX; end
            8'h79: begin op = OP_ADC; am = AM_ABY; end
            8'h61: begin op = OP_ADC; am = AM_IZX; end
            8'h71: begin op = OP_ADC; am = AM_IZY; end
            // AND
            8'h29: begin op = OP_AND; am = AM_IMM; end
            8'h25: begin op = OP_AND; am = AM_ZP;  end
            8'h35: begin op = OP_AND; am = AM_ZPX; end
            8'h2D: begin op = OP_AND; am = AM_ABS; end
            8'h3D: begin op = OP_AND; am = AM_ABX; end
            8'h39: begin op = OP_AND; am = AM_ABY; end
            8'h21: begin op = OP_AND; am = AM_IZX; end
            8'h31: begin op = OP_AND; am = AM_IZY; end
            // ASL
            8'h0A: begin op = OP_ASL; am = AM_ACC; end
            8'h06: begin op = OP_ASL; am = AM_ZP;  end
            8'h16: begin op = OP_ASL; am = AM_ZPX; end
            8'h0E: begin op = OP_ASL; am = AM_ABS; end
            8'h1E: begin op = OP_ASL; am = AM_ABX; end
            // the eight branches
            8'h10, 8'h30, 8'h50, 8'h70,
            8'h90, 8'hB0, 8'hD0, 8'hF0: begin op = OP_BRA; am = AM_REL; end
            // BIT
            8'h24: begin op = OP_BIT; am = AM_ZP;  end
            8'h2C: begin op = OP_BIT; am = AM_ABS; end
            // BRK
            8'h00: begin op = OP_BRK; am = AM_IMP; end
            // the flag instructions
            8'h18: begin op = OP_CLC; am = AM_IMP; end
            8'h38: begin op = OP_SEC; am = AM_IMP; end
            8'h58: begin op = OP_CLI; am = AM_IMP; end
            8'h78: begin op = OP_SEI; am = AM_IMP; end
            8'hB8: begin op = OP_CLV; am = AM_IMP; end
            8'hD8: begin op = OP_CLD; am = AM_IMP; end
            8'hF8: begin op = OP_SED; am = AM_IMP; end
            // CMP
            8'hC9: begin op = OP_CMP; am = AM_IMM; end
            8'hC5: begin op = OP_CMP; am = AM_ZP;  end
            8'hD5: begin op = OP_CMP; am = AM_ZPX; end
            8'hCD: begin op = OP_CMP; am = AM_ABS; end
            8'hDD: begin op = OP_CMP; am = AM_ABX; end
            8'hD9: begin op = OP_CMP; am = AM_ABY; end
            8'hC1: begin op = OP_CMP; am = AM_IZX; end
            8'hD1: begin op = OP_CMP; am = AM_IZY; end
            // CPX / CPY
            8'hE0: begin op = OP_CPX; am = AM_IMM; end
            8'hE4: begin op = OP_CPX; am = AM_ZP;  end
            8'hEC: begin op = OP_CPX; am = AM_ABS; end
            8'hC0: begin op = OP_CPY; am = AM_IMM; end
            8'hC4: begin op = OP_CPY; am = AM_ZP;  end
            8'hCC: begin op = OP_CPY; am = AM_ABS; end
            // DEC / DEX / DEY
            8'hC6: begin op = OP_DEC; am = AM_ZP;  end
            8'hD6: begin op = OP_DEC; am = AM_ZPX; end
            8'hCE: begin op = OP_DEC; am = AM_ABS; end
            8'hDE: begin op = OP_DEC; am = AM_ABX; end
            8'hCA: begin op = OP_DEX; am = AM_IMP; end
            8'h88: begin op = OP_DEY; am = AM_IMP; end
            // EOR
            8'h49: begin op = OP_EOR; am = AM_IMM; end
            8'h45: begin op = OP_EOR; am = AM_ZP;  end
            8'h55: begin op = OP_EOR; am = AM_ZPX; end
            8'h4D: begin op = OP_EOR; am = AM_ABS; end
            8'h5D: begin op = OP_EOR; am = AM_ABX; end
            8'h59: begin op = OP_EOR; am = AM_ABY; end
            8'h41: begin op = OP_EOR; am = AM_IZX; end
            8'h51: begin op = OP_EOR; am = AM_IZY; end
            // INC / INX / INY
            8'hE6: begin op = OP_INC; am = AM_ZP;  end
            8'hF6: begin op = OP_INC; am = AM_ZPX; end
            8'hEE: begin op = OP_INC; am = AM_ABS; end
            8'hFE: begin op = OP_INC; am = AM_ABX; end
            8'hE8: begin op = OP_INX; am = AM_IMP; end
            8'hC8: begin op = OP_INY; am = AM_IMP; end
            // JMP / JSR
            8'h4C: begin op = OP_JMP; am = AM_ABS; end
            8'h6C: begin op = OP_JMP; am = AM_IND; end
            8'h20: begin op = OP_JSR; am = AM_ABS; end
            // LDA
            8'hA9: begin op = OP_LDA; am = AM_IMM; end
            8'hA5: begin op = OP_LDA; am = AM_ZP;  end
            8'hB5: begin op = OP_LDA; am = AM_ZPX; end
            8'hAD: begin op = OP_LDA; am = AM_ABS; end
            8'hBD: begin op = OP_LDA; am = AM_ABX; end
            8'hB9: begin op = OP_LDA; am = AM_ABY; end
            8'hA1: begin op = OP_LDA; am = AM_IZX; end
            8'hB1: begin op = OP_LDA; am = AM_IZY; end
            // LDX / LDY
            8'hA2: begin op = OP_LDX; am = AM_IMM; end
            8'hA6: begin op = OP_LDX; am = AM_ZP;  end
            8'hB6: begin op = OP_LDX; am = AM_ZPY; end
            8'hAE: begin op = OP_LDX; am = AM_ABS; end
            8'hBE: begin op = OP_LDX; am = AM_ABY; end
            8'hA0: begin op = OP_LDY; am = AM_IMM; end
            8'hA4: begin op = OP_LDY; am = AM_ZP;  end
            8'hB4: begin op = OP_LDY; am = AM_ZPX; end
            8'hAC: begin op = OP_LDY; am = AM_ABS; end
            8'hBC: begin op = OP_LDY; am = AM_ABX; end
            // LSR
            8'h4A: begin op = OP_LSR; am = AM_ACC; end
            8'h46: begin op = OP_LSR; am = AM_ZP;  end
            8'h56: begin op = OP_LSR; am = AM_ZPX; end
            8'h4E: begin op = OP_LSR; am = AM_ABS; end
            8'h5E: begin op = OP_LSR; am = AM_ABX; end
            // NOP
            8'hEA: begin op = OP_NOP; am = AM_IMP; end
            // ORA
            8'h09: begin op = OP_ORA; am = AM_IMM; end
            8'h05: begin op = OP_ORA; am = AM_ZP;  end
            8'h15: begin op = OP_ORA; am = AM_ZPX; end
            8'h0D: begin op = OP_ORA; am = AM_ABS; end
            8'h1D: begin op = OP_ORA; am = AM_ABX; end
            8'h19: begin op = OP_ORA; am = AM_ABY; end
            8'h01: begin op = OP_ORA; am = AM_IZX; end
            8'h11: begin op = OP_ORA; am = AM_IZY; end
            // the stack instructions
            8'h48: begin op = OP_PHA; am = AM_IMP; end
            8'h08: begin op = OP_PHP; am = AM_IMP; end
            8'h68: begin op = OP_PLA; am = AM_IMP; end
            8'h28: begin op = OP_PLP; am = AM_IMP; end
            // ROL
            8'h2A: begin op = OP_ROL; am = AM_ACC; end
            8'h26: begin op = OP_ROL; am = AM_ZP;  end
            8'h36: begin op = OP_ROL; am = AM_ZPX; end
            8'h2E: begin op = OP_ROL; am = AM_ABS; end
            8'h3E: begin op = OP_ROL; am = AM_ABX; end
            // ROR
            8'h6A: begin op = OP_ROR; am = AM_ACC; end
            8'h66: begin op = OP_ROR; am = AM_ZP;  end
            8'h76: begin op = OP_ROR; am = AM_ZPX; end
            8'h6E: begin op = OP_ROR; am = AM_ABS; end
            8'h7E: begin op = OP_ROR; am = AM_ABX; end
            // RTI / RTS
            8'h40: begin op = OP_RTI; am = AM_IMP; end
            8'h60: begin op = OP_RTS; am = AM_IMP; end
            // SBC
            8'hE9: begin op = OP_SBC; am = AM_IMM; end
            8'hE5: begin op = OP_SBC; am = AM_ZP;  end
            8'hF5: begin op = OP_SBC; am = AM_ZPX; end
            8'hED: begin op = OP_SBC; am = AM_ABS; end
            8'hFD: begin op = OP_SBC; am = AM_ABX; end
            8'hF9: begin op = OP_SBC; am = AM_ABY; end
            8'hE1: begin op = OP_SBC; am = AM_IZX; end
            8'hF1: begin op = OP_SBC; am = AM_IZY; end
            // STA
            8'h85: begin op = OP_STA; am = AM_ZP;  end
            8'h95: begin op = OP_STA; am = AM_ZPX; end
            8'h8D: begin op = OP_STA; am = AM_ABS; end
            8'h9D: begin op = OP_STA; am = AM_ABX; end
            8'h99: begin op = OP_STA; am = AM_ABY; end
            8'h81: begin op = OP_STA; am = AM_IZX; end
            8'h91: begin op = OP_STA; am = AM_IZY; end
            // STX / STY
            8'h86: begin op = OP_STX; am = AM_ZP;  end
            8'h96: begin op = OP_STX; am = AM_ZPY; end
            8'h8E: begin op = OP_STX; am = AM_ABS; end
            8'h84: begin op = OP_STY; am = AM_ZP;  end
            8'h94: begin op = OP_STY; am = AM_ZPX; end
            8'h8C: begin op = OP_STY; am = AM_ABS; end
            // the transfers
            8'hAA: begin op = OP_TAX; am = AM_IMP; end
            8'hA8: begin op = OP_TAY; am = AM_IMP; end
            8'hBA: begin op = OP_TSX; am = AM_IMP; end
            8'h8A: begin op = OP_TXA; am = AM_IMP; end
            8'h9A: begin op = OP_TXS; am = AM_IMP; end
            8'h98: begin op = OP_TYA; am = AM_IMP; end
            default: begin op = OP_NOP; am = AM_IMP; end
        endcase
    end

    // What the instruction does with the byte at its effective address.
    reg [1:0] kind;
    always @(*) begin
        case (op)
            OP_ORA, OP_AND, OP_EOR, OP_ADC, OP_SBC, OP_CMP, OP_CPX,
            OP_CPY, OP_BIT, OP_LDA, OP_LDX, OP_LDY: kind = K_READ;
            OP_STA, OP_STX, OP_STY:                 kind = K_WRITE;
            OP_ASL, OP_LSR, OP_ROL, OP_ROR, OP_INC, OP_DEC:
                kind = (am == AM_ACC) ? K_NONE : K_RMW;
            default:                                kind = K_NONE;
        endcase
    end

    // An indexed write or read-modify-write always spends the cycle a
    // read only spends when the index carried, so STA $1234,X is five
    // cycles and LDA $1234,X is four.
    wire needs_fix = ((kind == K_WRITE) | (kind == K_RMW))
                   & ((am == AM_ABX) | (am == AM_ABY) | (am == AM_IZY));

    // X indexes everything but the ,Y modes.
    wire [7:0] idx = ((am == AM_ZPY) | (am == AM_ABY) | (am == AM_IZY)) ? y_r : x_r;
    wire [8:0] idx_sum = {1'b0, adl} + {1'b0, idx};
    wire [7:0] zp_sum  = bl + idx;

    wire stack_op = (op == OP_PHA) | (op == OP_PHP) | (op == OP_PLA)
                  | (op == OP_PLP) | (op == OP_RTS) | (op == OP_RTI);

    // The four index counters, so INX / DEX / INY / DEY can set their
    // flags off a named value rather than off a repeated expression.
    wire [7:0] x_inc = x_r + 8'd1;
    wire [7:0] x_dec = x_r - 8'd1;
    wire [7:0] y_inc = y_r + 8'd1;
    wire [7:0] y_dec = y_r - 8'd1;

    // -----------------------------------------------------------------
    // Arithmetic and logic.
    // -----------------------------------------------------------------
    wire [7:0] m = din;

    wire [7:0] r_ora = a_r | m;
    wire [7:0] r_and = a_r & m;
    wire [7:0] r_eor = a_r ^ m;

    wire [8:0] bin_add = {1'b0, a_r} + {1'b0, m} + {8'd0, p_c};
    wire [8:0] bin_sub = {1'b0, a_r} - {1'b0, m} - {8'd0, ~p_c};
    wire       bin_v   = (~(a_r[7] ^ m[7])) & (a_r[7] ^ bin_add[7]);
    wire       sbc_v   = (a_r[7] ^ m[7]) & (a_r[7] ^ bin_sub[7]);

    wire [8:0] cmp_a = {1'b0, a_r} - {1'b0, m};
    wire [8:0] cmp_x = {1'b0, x_r} - {1'b0, m};
    wire [8:0] cmp_y = {1'b0, y_r} - {1'b0, m};

    wire [7:0] adc_res;
    wire       adc_c;
    wire       adc_n;
    wire       adc_v;
    wire [7:0] sbc_res;

    generate
        if (DECIMAL_MODE != 0) begin : g_decimal
            // ADC: the low nibble is corrected first, N and V are read
            // off the intermediate that correction leaves, and only then
            // is the high nibble corrected.
            wire [9:0] dlo0 = {6'd0, a_r[3:0]} + {6'd0, m[3:0]} + {9'd0, p_c};
            wire       dadj = (dlo0 > 10'd9);
            wire [9:0] dlo  = dadj ? (dlo0 + 10'd6) : dlo0;
            wire [9:0] dhi0 = {2'd0, a_r[7:4], 4'd0} + {2'd0, m[7:4], 4'd0}
                            + (dadj ? 10'h010 : 10'h000);
            wire [9:0] dhi  = (dhi0 > 10'h090) ? (dhi0 + 10'h060) : dhi0;

            assign adc_res = p_d ? {dhi[7:4], dlo[3:0]} : bin_add[7:0];
            assign adc_c   = p_d ? dhi[8]               : bin_add[8];
            assign adc_n   = p_d ? dhi0[7]              : bin_add[7];
            assign adc_v   = p_d ? ((~(a_r[7] ^ m[7])) & (a_r[7] ^ dhi0[7])) : bin_v;

            // SBC: only the accumulator differs; every flag is the
            // binary subtraction's.
            wire [4:0] slo0 = {1'b0, a_r[3:0]} - {1'b0, m[3:0]} - {4'd0, ~p_c};
            wire [4:0] slo  = slo0[4] ? (slo0 - 5'd6) : slo0;
            wire [4:0] shi0 = {1'b0, a_r[7:4]} - {1'b0, m[7:4]} - {4'd0, slo0[4]};
            wire [4:0] shi  = shi0[4] ? (shi0 - 5'd6) : shi0;

            assign sbc_res = p_d ? {shi[3:0], slo[3:0]} : bin_sub[7:0];
        end else begin : g_binary
            assign adc_res = bin_add[7:0];
            assign adc_c   = bin_add[8];
            assign adc_n   = bin_add[7];
            assign adc_v   = bin_v;
            assign sbc_res = bin_sub[7:0];
        end
    endgenerate

    // The shifts, rotates and the two counters, over the accumulator or
    // over the byte a read-modify-write latched.
    wire [7:0] sh_src = (am == AM_ACC) ? a_r : md;
    reg  [7:0] sh_res;
    reg        sh_c;
    always @(*) begin
        sh_c   = p_c;
        sh_res = sh_src - 8'd1;
        case (op)
            OP_ASL: begin sh_res = {sh_src[6:0], 1'b0}; sh_c = sh_src[7]; end
            OP_ROL: begin sh_res = {sh_src[6:0], p_c};  sh_c = sh_src[7]; end
            OP_LSR: begin sh_res = {1'b0, sh_src[7:1]}; sh_c = sh_src[0]; end
            OP_ROR: begin sh_res = {p_c, sh_src[7:1]};  sh_c = sh_src[0]; end
            OP_INC: sh_res = sh_src + 8'd1;
            default: sh_res = sh_src - 8'd1;
        endcase
    end

    // -----------------------------------------------------------------
    // Branches.
    // -----------------------------------------------------------------
    // The opcode of a branch is ffv10000: bits 7:6 name the flag and
    // bit 5 the value it has to have.
    reg branch_flag;
    always @(*) begin
        case (opc[7:6])
            2'b00:   branch_flag = p_n;
            2'b01:   branch_flag = p_v;
            2'b10:   branch_flag = p_c;
            default: branch_flag = p_z;
        endcase
    end
    wire branch_taken = (branch_flag == opc[5]);

    wire [8:0] br_sum   = {1'b0, pc[7:0]} + {1'b0, md};
    wire       br_cross = br_sum[8] ^ md[7];

    // -----------------------------------------------------------------
    // The status byte as the stack sees it, and the interrupt vectors.
    // -----------------------------------------------------------------
    wire [7:0] p_brk = {p_n, p_v, 1'b1, 1'b1, p_d, p_i, p_z, p_c};
    wire [7:0] p_int = {p_n, p_v, 1'b1, 1'b0, p_d, p_i, p_z, p_c};
    wire       is_res = (int_kind == K_RES);
    wire [7:0] push_d = (op == OP_PHP) ? p_brk : a_r;
    wire [7:0] int_d  = (int_kind == K_BRK) ? p_brk : p_int;

    reg [15:0] vec;
    always @(*) begin
        case (int_kind)
            K_NMI:   vec = 16'hFFFA;
            K_RES:   vec = 16'hFFFC;
            default: vec = 16'hFFFE;
        endcase
    end

    reg [7:0] store_d;
    always @(*) begin
        case (op)
            OP_STX:  store_d = x_r;
            OP_STY:  store_d = y_r;
            default: store_d = a_r;
        endcase
    end

    // -----------------------------------------------------------------
    // The next state. An interrupt is decided here, at the end of an
    // instruction and with the flags as they were before it — which is
    // what gives CLI, SEI and PLP their one-instruction delay.
    // -----------------------------------------------------------------
    wire       take_int  = nmi_pend | (irq & ~p_i);
    wire [5:0] nxt_fetch = take_int ? S_INTD : S_FETCH;

    wire [5:0] after_addr = (kind == K_READ)  ? S_READ
                          : needs_fix         ? S_WRFIX
                          : (kind == K_WRITE) ? S_WRITE
                                              : S_RMWR;

    reg [5:0] nxt;
    reg       last_cycle;
    always @(*) begin
        nxt        = S_FETCH;
        last_cycle = 1'b0;
        case (st)
            S_FETCH: begin
                if (op == OP_BRK)      nxt = S_BRKPC;
                else if (op == OP_JSR) nxt = S_ABSL;
                else if (stack_op)     nxt = S_STKD;
                else begin
                    case (am)
                        AM_IMP, AM_ACC:                        nxt = S_IMPL;
                        AM_IMM:                                nxt = S_IMM;
                        AM_REL:                                nxt = S_REL;
                        AM_ZP, AM_ZPX, AM_ZPY, AM_IZX, AM_IZY: nxt = S_ZP;
                        default:                               nxt = S_ABSL;
                    endcase
                end
            end
            S_IMPL: begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_IMM:  begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_REL:
                if (branch_taken) nxt = S_BR1;
                else begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_BR1:
                if (br_cross) nxt = S_BR2;
                else begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_BR2: begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_ZP: begin
                case (am)
                    AM_ZPX, AM_ZPY, AM_IZX: nxt = S_ZPIDX;
                    AM_IZY:                 nxt = S_INDL;
                    default:                nxt = after_addr;
                endcase
            end
            S_ZPIDX: nxt = (am == AM_IZX) ? S_INDL : after_addr;
            S_INDL:  nxt = S_INDH;
            S_INDH:  nxt = after_addr;
            S_ABSL:  nxt = (op == OP_JSR) ? S_JSRD : S_ABSH;
            S_ABSH: begin
                if ((op == OP_JMP) && (am == AM_ABS)) begin
                    nxt = nxt_fetch; last_cycle = 1'b1;
                end else if (am == AM_IND) begin
                    nxt = S_JMPL;
                end else begin
                    nxt = after_addr;
                end
            end
            S_JMPL: nxt = S_JMPH;
            S_JMPH: begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_READ:
                if (fix) nxt = S_READ;
                else begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_WRFIX:  nxt = (kind == K_WRITE) ? S_WRITE : S_RMWR;
            S_WRITE:  begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_RMWR:   nxt = S_RMWW1;
            S_RMWW1:  nxt = S_RMWW2;
            S_RMWW2:  begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_STKD:   nxt = ((op == OP_PHA) | (op == OP_PHP)) ? S_PUSH : S_PULLD;
            S_PUSH:   begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_PULLD:  nxt = ((op == OP_PLA) | (op == OP_PLP)) ? S_PULL
                          : (op == OP_RTI)                    ? S_RTIP
                                                              : S_RTSPL;
            S_PULL:   begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_RTIP:   nxt = S_RTSPL;
            S_RTSPL:  nxt = S_RTSPH;
            S_RTSPH:
                if (op == OP_RTI) begin nxt = nxt_fetch; last_cycle = 1'b1; end
                else nxt = S_RTSPC;
            S_RTSPC:  begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_JSRD:   nxt = S_JSRPH;
            S_JSRPH:  nxt = S_JSRPL;
            S_JSRPL:  nxt = S_JSRH;
            S_JSRH:   begin nxt = nxt_fetch; last_cycle = 1'b1; end
            S_BRKPC:  nxt = S_INTPH;
            S_INTD:   nxt = S_INTD2;
            S_INTD2:  nxt = S_INTPH;
            S_INTPH:  nxt = S_INTPL;
            S_INTPL:  nxt = S_INTP;
            S_INTP:   nxt = S_VECL;
            S_VECL:   nxt = S_VECH;
            default:  begin nxt = nxt_fetch; last_cycle = 1'b1; end  // S_VECH
        endcase
    end

    // -----------------------------------------------------------------
    // The bus. Exactly one access a cycle, at the address the original
    // would put on its pins.
    // -----------------------------------------------------------------
    reg [15:0] addr_c;
    reg        we_c;
    reg [7:0]  dout_c;
    always @(*) begin
        addr_c = pc;
        we_c   = 1'b0;
        dout_c = a_r;
        case (st)
            S_ZPIDX, S_INDL: addr_c = {8'h00, bl};
            // The pointer's high byte comes from the *next* zero-page
            // address, which wraps inside page zero.
            S_INDH:          addr_c = {8'h00, bl + 8'd1};
            S_JMPL:          addr_c = {adh, adl};
            // The indirect JMP page bug: the pointer's low byte is
            // incremented and there is no carry into its high byte.
            S_JMPH:          addr_c = {adh, adl + 8'd1};
            S_READ, S_WRFIX, S_RMWR: addr_c = {adh, adl};
            S_WRITE: begin addr_c = {adh, adl}; we_c = 1'b1; dout_c = store_d; end
            S_RMWW1: begin addr_c = {adh, adl}; we_c = 1'b1; dout_c = md;      end
            S_RMWW2: begin addr_c = {adh, adl}; we_c = 1'b1; dout_c = sh_res;  end
            S_PULLD, S_PULL, S_RTIP, S_RTSPL, S_RTSPH, S_JSRD:
                addr_c = {8'h01, s_r};
            S_PUSH:  begin addr_c = {8'h01, s_r}; we_c = 1'b1;   dout_c = push_d;   end
            S_JSRPH: begin addr_c = {8'h01, s_r}; we_c = 1'b1;   dout_c = pc[15:8]; end
            S_JSRPL: begin addr_c = {8'h01, s_r}; we_c = 1'b1;   dout_c = pc[7:0];  end
            // The reset sequence goes through the motions of the three
            // pushes but reads, exactly as the original does.
            S_INTPH: begin addr_c = {8'h01, s_r}; we_c = ~is_res; dout_c = pc[15:8]; end
            S_INTPL: begin addr_c = {8'h01, s_r}; we_c = ~is_res; dout_c = pc[7:0];  end
            S_INTP:  begin addr_c = {8'h01, s_r}; we_c = ~is_res; dout_c = int_d;    end
            S_VECL:  addr_c = vec;
            S_VECH:  addr_c = vec + 16'd1;
            default: addr_c = pc;
        endcase
    end

    assign addr = addr_c;
    assign we   = we_c;
    assign dout = dout_c;
    assign sync = (st == S_FETCH);

    // The cycle that carries out a read instruction: the immediate
    // operand, or the operand at the effective address once any page
    // crossing has been paid for.
    wire rd_exec = (st == S_IMM) | ((st == S_READ) & ~fix);

    // -----------------------------------------------------------------
    // Trace.
    // -----------------------------------------------------------------
    reg [15:0] dbg_pc_q;
    reg        dbg_retire_q;
    reg        dbg_trap_q;
    assign dbg_pc     = dbg_pc_q;
    assign dbg_retire = dbg_retire_q;
    assign dbg_trap   = dbg_trap_q;

    // -----------------------------------------------------------------
    // The sequential core.
    // -----------------------------------------------------------------
    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            a_r      <= 8'h00;
            x_r      <= 8'h00;
            y_r      <= 8'h00;
            s_r      <= 8'h00;
            p_n      <= 1'b0;
            p_v      <= 1'b0;
            p_d      <= 1'b0;
            p_i      <= 1'b0;
            p_z      <= 1'b0;
            p_c      <= 1'b0;
            pc       <= 16'h0000;
            ir       <= 8'h00;
            md       <= 8'h00;
            bl       <= 8'h00;
            adl      <= 8'h00;
            adh      <= 8'h00;
            fix      <= 1'b0;
            // Releasing reset runs the seven-cycle reset sequence, which
            // is the interrupt sequence with reads for the pushes.
            st       <= S_INTD;
            int_kind <= K_RES;
            nmi_q    <= 1'b0;
            nmi_pend <= 1'b0;
            op_pc    <= 16'h0000;
            dbg_pc_q <= 16'h0000;
            dbg_retire_q <= 1'b0;
            dbg_trap_q   <= 1'b0;
        end else begin
            dbg_retire_q <= ready & last_cycle & (st != S_VECH);
            dbg_trap_q   <= ready & (st == S_VECH);

            if (ready) begin
                st <= nxt;
                if (last_cycle) dbg_pc_q <= op_pc;

                case (st)
                    S_FETCH: begin
                        ir    <= din;
                        op_pc <= pc;
                        pc    <= pc + 16'd1;
                        if (op == OP_BRK) int_kind <= K_BRK;
                    end
                    S_IMM: pc <= pc + 16'd1;
                    S_REL: begin
                        md <= din;
                        pc <= pc + 16'd1;
                    end
                    S_BR1: pc <= {pc[15:8], br_sum[7:0]};
                    S_BR2: pc <= {md[7] ? (pc[15:8] - 8'd1) : (pc[15:8] + 8'd1), pc[7:0]};
                    S_ZP: begin
                        bl  <= din;
                        adl <= din;
                        adh <= 8'h00;
                        fix <= 1'b0;
                        pc  <= pc + 16'd1;
                    end
                    S_ZPIDX: begin
                        // Zero-page indexing wraps inside page zero.
                        bl  <= zp_sum;
                        adl <= zp_sum;
                        adh <= 8'h00;
                        fix <= 1'b0;
                    end
                    S_INDL: adl <= din;
                    S_INDH: begin
                        adh <= din;
                        if (am == AM_IZY) begin
                            adl <= idx_sum[7:0];
                            fix <= idx_sum[8];
                        end
                    end
                    S_ABSL: begin
                        adl <= din;
                        fix <= 1'b0;
                        pc  <= pc + 16'd1;
                    end
                    S_ABSH: begin
                        adh <= din;
                        pc  <= pc + 16'd1;
                        if ((op == OP_JMP) && (am == AM_ABS)) begin
                            pc <= {din, adl};
                        end else if ((am == AM_ABX) || (am == AM_ABY)) begin
                            adl <= idx_sum[7:0];
                            fix <= idx_sum[8];
                        end
                    end
                    S_JMPL: md <= din;
                    S_JMPH: pc <= {din, md};
                    S_READ: if (fix) begin
                        adh <= adh + 8'd1;
                        fix <= 1'b0;
                    end
                    S_WRFIX: if (fix) begin
                        adh <= adh + 8'd1;
                        fix <= 1'b0;
                    end
                    S_RMWR: md <= din;
                    S_PUSH: s_r <= s_r - 8'd1;
                    S_PULLD: s_r <= s_r + 8'd1;
                    S_PULL: begin
                        if (op == OP_PLA) begin
                            a_r <= din;
                            p_n <= din[7];
                            p_z <= (din == 8'h00);
                        end else begin
                            // PLP ignores bits 4 and 5: B is not a flag.
                            p_n <= din[7];
                            p_v <= din[6];
                            p_d <= din[3];
                            p_i <= din[2];
                            p_z <= din[1];
                            p_c <= din[0];
                        end
                    end
                    S_RTIP: begin
                        s_r <= s_r + 8'd1;
                        p_n <= din[7];
                        p_v <= din[6];
                        p_d <= din[3];
                        p_i <= din[2];
                        p_z <= din[1];
                        p_c <= din[0];
                    end
                    S_RTSPL: begin
                        md  <= din;
                        s_r <= s_r + 8'd1;
                    end
                    S_RTSPH: pc <= {din, md};
                    S_RTSPC: pc <= pc + 16'd1;
                    S_JSRPH: s_r <= s_r - 8'd1;
                    S_JSRPL: s_r <= s_r - 8'd1;
                    S_JSRH:  pc <= {din, adl};
                    S_BRKPC: pc <= pc + 16'd1;
                    S_INTPH: s_r <= s_r - 8'd1;
                    S_INTPL: s_r <= s_r - 8'd1;
                    S_INTP: begin
                        s_r <= s_r - 8'd1;
                        p_i <= 1'b1;
                    end
                    S_VECL: md <= din;
                    S_VECH: pc <= {din, md};
                    // S_IMPL, S_STKD, S_JSRD, S_INTD and S_INTD2 move
                    // no register of their own.
                    default: pc <= pc;
                endcase

                // The implied and accumulator instructions.
                if (st == S_IMPL) begin
                    case (op)
                        OP_TAX: begin x_r <= a_r; p_n <= a_r[7]; p_z <= (a_r == 8'h00); end
                        OP_TAY: begin y_r <= a_r; p_n <= a_r[7]; p_z <= (a_r == 8'h00); end
                        OP_TXA: begin a_r <= x_r; p_n <= x_r[7]; p_z <= (x_r == 8'h00); end
                        OP_TYA: begin a_r <= y_r; p_n <= y_r[7]; p_z <= (y_r == 8'h00); end
                        OP_TSX: begin x_r <= s_r; p_n <= s_r[7]; p_z <= (s_r == 8'h00); end
                        // TXS is the one transfer that sets no flags.
                        OP_TXS: s_r <= x_r;
                        OP_INX: begin
                            x_r <= x_inc; p_n <= x_inc[7]; p_z <= (x_inc == 8'h00);
                        end
                        OP_DEX: begin
                            x_r <= x_dec; p_n <= x_dec[7]; p_z <= (x_dec == 8'h00);
                        end
                        OP_INY: begin
                            y_r <= y_inc; p_n <= y_inc[7]; p_z <= (y_inc == 8'h00);
                        end
                        OP_DEY: begin
                            y_r <= y_dec; p_n <= y_dec[7]; p_z <= (y_dec == 8'h00);
                        end
                        OP_CLC: p_c <= 1'b0;
                        OP_SEC: p_c <= 1'b1;
                        OP_CLI: p_i <= 1'b0;
                        OP_SEI: p_i <= 1'b1;
                        OP_CLV: p_v <= 1'b0;
                        OP_CLD: p_d <= 1'b0;
                        OP_SED: p_d <= 1'b1;
                        OP_ASL, OP_LSR, OP_ROL, OP_ROR: begin
                            a_r <= sh_res;
                            p_c <= sh_c;
                            p_n <= sh_res[7];
                            p_z <= (sh_res == 8'h00);
                        end
                        // NOP, and every undocumented opcode.
                        default: p_c <= p_c;
                    endcase
                end

                // The read instructions.
                if (rd_exec) begin
                    case (op)
                        OP_LDA: begin a_r <= m; p_n <= m[7]; p_z <= (m == 8'h00); end
                        OP_LDX: begin x_r <= m; p_n <= m[7]; p_z <= (m == 8'h00); end
                        OP_LDY: begin y_r <= m; p_n <= m[7]; p_z <= (m == 8'h00); end
                        OP_ORA: begin
                            a_r <= r_ora; p_n <= r_ora[7]; p_z <= (r_ora == 8'h00);
                        end
                        OP_AND: begin
                            a_r <= r_and; p_n <= r_and[7]; p_z <= (r_and == 8'h00);
                        end
                        OP_EOR: begin
                            a_r <= r_eor; p_n <= r_eor[7]; p_z <= (r_eor == 8'h00);
                        end
                        OP_ADC: begin
                            a_r <= adc_res;
                            p_c <= adc_c;
                            p_n <= adc_n;
                            p_v <= adc_v;
                            // Z is the binary sum's even in decimal mode.
                            p_z <= (bin_add[7:0] == 8'h00);
                        end
                        OP_SBC: begin
                            a_r <= sbc_res;
                            p_c <= ~bin_sub[8];
                            p_n <= bin_sub[7];
                            p_v <= sbc_v;
                            p_z <= (bin_sub[7:0] == 8'h00);
                        end
                        OP_CMP: begin
                            p_c <= ~cmp_a[8];
                            p_n <= cmp_a[7];
                            p_z <= (cmp_a[7:0] == 8'h00);
                        end
                        OP_CPX: begin
                            p_c <= ~cmp_x[8];
                            p_n <= cmp_x[7];
                            p_z <= (cmp_x[7:0] == 8'h00);
                        end
                        OP_CPY: begin
                            p_c <= ~cmp_y[8];
                            p_n <= cmp_y[7];
                            p_z <= (cmp_y[7:0] == 8'h00);
                        end
                        OP_BIT: begin
                            p_n <= m[7];
                            p_v <= m[6];
                            p_z <= ((a_r & m) == 8'h00);
                        end
                        default: p_c <= p_c;
                    endcase
                end

                // A read-modify-write sets its flags as it writes the
                // result; `sh_c` is the old carry for INC and DEC, so
                // neither disturbs it.
                if (st == S_RMWW2) begin
                    p_c <= sh_c;
                    p_n <= sh_res[7];
                    p_z <= (sh_res == 8'h00);
                end

                // An interrupt is decided at the end of an instruction.
                if (last_cycle && take_int) begin
                    if (nmi_pend) begin
                        int_kind <= K_NMI;
                        nmi_pend <= 1'b0;
                    end else begin
                        int_kind <= K_IRQ;
                    end
                end
            end

            // The NMI edge is watched whatever the bus is doing, and a
            // fresh edge in the cycle one is serviced is kept.
            nmi_q <= nmi;
            if (nmi & ~nmi_q) nmi_pend <= 1'b1;
        end
    end
endmodule
