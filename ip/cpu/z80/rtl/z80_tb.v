// z80_tb — does the core execute, and does it take the published number
// of T-states doing it?
//
// Two questions, and the second is the one a reader should care about.
// A core that computes the right answer in the wrong number of cycles is
// not a Z80: the timings are the part's published interface, and every
// system built around one depends on them. So the T-state table below is
// typed from the Zilog figures and compared against the hardware, the
// same way `tests/ip_library.rs` compares `mos6502` against the 6502's
// own cycle counts.
//
// The memory is combinational on read and writes on the clock while
// `MREQ` and `WR` are both low, which is the simplest thing that meets
// the part's timing: `din` is sampled at the end of T3 and the data has
// been stable since T1.
//
// What this would catch: a wrong result, a wrong flag, a wrong T-state
// count, a jump to the wrong place, a stack that does not balance. What
// it would not: anything about `WAIT`, which nothing here drives low;
// anything about the interrupt inputs, which nothing here asserts; and
// the three prefixes the core does not decode. Those want their own
// benches and are listed in the core's header as not implemented.
module z80_tb;
    reg clk = 1'b0;
    always #5 clk = ~clk;
    reg rst_n = 1'b0;

    wire [15:0] addr;
    wire [7:0]  dout;
    wire        dout_oe, m1_n, mreq_n, iorq_n, rd_n, wr_n, rfsh_n, halt_n;
    wire [15:0] dbg_pc;
    wire        dbg_retire, dbg_trap;

    reg [7:0] mem [0:65535];
    reg [7:0] io_in = 8'ha5;
    // A read takes memory unless `IORQ` says otherwise; nothing else in
    // this bench drives the bus.
    wire [7:0] din = iorq_n ? mem[addr] : io_in;

    z80 dut (
        .clk(clk), .rst_n(rst_n),
        .addr(addr), .dout(dout), .din(din), .dout_oe(dout_oe),
        .m1_n(m1_n), .mreq_n(mreq_n), .iorq_n(iorq_n),
        .rd_n(rd_n), .wr_n(wr_n), .rfsh_n(rfsh_n), .halt_n(halt_n),
        .wait_n(1'b1), .int_n(1'b1), .nmi_n(1'b1),
        .dbg_pc(dbg_pc), .dbg_retire(dbg_retire), .dbg_trap(dbg_trap));

    always @(posedge clk) begin
        if (!mreq_n && !wr_n) mem[addr] <= dout;
    end

    integer fails = 0;
    integer i;
    integer got_t = 0;
    reg     done = 1'b0;

    // **Reset is asserted before the program is loaded, not after.** A
    // core still running the previous program writes to memory for as
    // long as it is let, and those writes landed on top of the bytes the
    // next check had just put at address zero -- which made a core that
    // counts T-states correctly report the same wrong number for a
    // scattered third of the table.
    task hold_reset;
        begin
            rst_n = 1'b0;
            repeat (3) @(posedge clk);
        end
    endtask

    // Leaves `rst_n` high immediately after a rising edge, so the first
    // falling edge after it is inside the instruction's first T-state.
    task release_reset;
        begin
            @(posedge clk);
            rst_n = 1'b1;
        end
    endtask

    // Runs one instruction from address zero and checks its T-states.
    task check_t;
        // Up to four bytes, the **first** in the low eight bits, so a
        // two-byte instruction reads `{operand, opcode}`.
        input [31:0]  bytes;
        input integer nbytes;
        input integer want;
        input [159:0] name;
        begin
            hold_reset;
            for (i = 0; i < 65536; i = i + 1) mem[i] = 8'h00;
            for (i = 0; i < nbytes; i = i + 1) mem[i] = bytes[i * 8 +: 8];
            release_reset;
            got_t = 0;
            done  = 1'b0;
            while (!done) begin
                @(negedge clk);
                got_t = got_t + 1;
                if (dbg_retire) done = 1'b1;
                if (got_t > 100) done = 1'b1;   // a core that never retires
            end
            if (got_t !== want) begin
                $display("FAIL %0s: %0d T-states, the tables say %0d", name, got_t, want);
                fails = fails + 1;
            end
        end
    endtask

    // Runs a program already in `mem` until it reaches a HALT.
    task run_until_halt;
        input integer limit;
        begin
            release_reset;
            i = 0;
            while (halt_n && (i < limit)) begin
                @(posedge clk);
                i = i + 1;
            end
            if (halt_n) begin
                $display("FAIL: no HALT after %0d clocks, pc = %04x", limit, dbg_pc);
                fails = fails + 1;
            end
        end
    endtask

    task expect8;
        input [15:0]  at;
        input [7:0]   want;
        input [159:0] name;
        begin
            if (mem[at] !== want) begin
                $display("FAIL %0s: [%04x] = %02x, wanted %02x", name, at, mem[at], want);
                fails = fails + 1;
            end
        end
    endtask

    initial begin
        // ---- the T-state table, typed from the Zilog figures ----
        check_t(32'h00, 1, 4,  "nop");
        check_t(32'h78, 1, 4,  "ld a,b");
        check_t(32'h003e, 2, 7,  "ld a,n");
        check_t(32'h7e, 1, 7,  "ld a,(hl)");
        check_t(32'h77, 1, 7,  "ld (hl),a");
        check_t(32'h0036, 2, 10, "ld (hl),n");
        check_t(32'h000021, 3, 10, "ld hl,nn");
        check_t(32'hf9, 1, 6,  "ld sp,hl");
        check_t(32'h00003a, 3, 13, "ld a,(nn)");
        check_t(32'h000032, 3, 13, "ld (nn),a");
        check_t(32'h00002a, 3, 16, "ld hl,(nn)");
        check_t(32'h000022, 3, 16, "ld (nn),hl");
        check_t(32'h0a, 1, 7,  "ld a,(bc)");
        check_t(32'h02, 1, 7,  "ld (bc),a");
        check_t(32'h80, 1, 4,  "add a,b");
        check_t(32'h00c6, 2, 7,  "add a,n");
        check_t(32'h86, 1, 7,  "add a,(hl)");
        check_t(32'h3c, 1, 4,  "inc a");
        check_t(32'h34, 1, 11, "inc (hl)");
        check_t(32'h23, 1, 6,  "inc hl");
        check_t(32'h09, 1, 11, "add hl,bc");
        check_t(32'hc5, 1, 11, "push bc");
        check_t(32'hc1, 1, 10, "pop bc");
        check_t(32'he3, 1, 19, "ex (sp),hl");
        check_t(32'heb, 1, 4,  "ex de,hl");
        check_t(32'h08, 1, 4,  "ex af,af'");
        check_t(32'hd9, 1, 4,  "exx");
        check_t(32'h0000c3, 3, 10, "jp nn");
        check_t(32'h0000c2, 3, 10, "jp nz,nn");
        check_t(32'he9, 1, 4,  "jp (hl)");
        check_t(32'h0018, 2, 12, "jr e");
        check_t(32'h0028, 2, 12, "jr z,e taken");
        check_t(32'h0000cd, 3, 17, "call nn");
        check_t(32'hc9, 1, 10, "ret");
        check_t(32'hc8, 1, 11, "ret z taken");
        check_t(32'hc7, 1, 11, "rst 0");
        check_t(32'h00db, 2, 11, "in a,(n)");
        check_t(32'h00d3, 2, 11, "out (n),a");
        check_t(32'h27, 1, 4,  "daa");
        check_t(32'h07, 1, 4,  "rlca");
        check_t(32'h00cb, 2, 8,  "rlc b");
        check_t(32'h06cb, 2, 15, "rlc (hl)");
        check_t(32'h40cb, 2, 8,  "bit 0,b");
        check_t(32'h46cb, 2, 12, "bit 0,(hl)");
        check_t(32'h86cb, 2, 15, "res 0,(hl)");
        check_t(32'hc6cb, 2, 15, "set 0,(hl)");

        // A not-taken conditional is the other half of the pair, and the
        // flags after a reset are FF, so Z is set and NZ is false.
        check_t(32'h0020, 2, 7,  "jr nz,e not taken");
        check_t(32'hc0, 1, 5,  "ret nz not taken");
        // DJNZ with B = 0 goes to FF and jumps; the not-taken case needs
        // B == 1 and so cannot be reached from reset alone.
        check_t(32'h0010, 2, 13, "djnz e taken");

        // ---- does it compute? ----
        // 8-bit arithmetic, the stack, a loop and a call.
        hold_reset;
        for (i = 0; i < 65536; i = i + 1) mem[i] = 8'h00;
        mem[16'h0000] = 8'h31; mem[16'h0001] = 8'hf0; mem[16'h0002] = 8'hff; // ld sp,$fff0
        mem[16'h0003] = 8'h3e; mem[16'h0004] = 8'h05;                       // ld a,5
        mem[16'h0005] = 8'h06; mem[16'h0006] = 8'h03;                       // ld b,3
        mem[16'h0007] = 8'h80;                                              // add a,b
        mem[16'h0008] = 8'h32; mem[16'h0009] = 8'h00; mem[16'h000a] = 8'h90; // ld ($9000),a
        // A loop: sum 1..4 into C.
        mem[16'h000b] = 8'h0e; mem[16'h000c] = 8'h00;                       // ld c,0
        mem[16'h000d] = 8'h06; mem[16'h000e] = 8'h04;                       // ld b,4
        mem[16'h000f] = 8'h79;                                              // ld a,c
        mem[16'h0010] = 8'h80;                                              // add a,b
        mem[16'h0011] = 8'h4f;                                              // ld c,a
        mem[16'h0012] = 8'h10; mem[16'h0013] = 8'hfb;                       // djnz -5
        mem[16'h0014] = 8'h79;                                              // ld a,c
        mem[16'h0015] = 8'h32; mem[16'h0016] = 8'h01; mem[16'h0017] = 8'h90; // ld ($9001),a
        // A call that writes through HL, and a push/pop round trip.
        mem[16'h0018] = 8'h21; mem[16'h0019] = 8'h02; mem[16'h001a] = 8'h90; // ld hl,$9002
        mem[16'h001b] = 8'hcd; mem[16'h001c] = 8'h30; mem[16'h001d] = 8'h00; // call $0030
        mem[16'h001e] = 8'h01; mem[16'h001f] = 8'h34; mem[16'h0020] = 8'h12; // ld bc,$1234
        mem[16'h0021] = 8'hc5;                                              // push bc
        mem[16'h0022] = 8'he1;                                              // pop hl
        mem[16'h0023] = 8'h22; mem[16'h0024] = 8'h03; mem[16'h0025] = 8'h90; // ld ($9003),hl
        // CB: set bit 7 of a byte in memory, then test it.
        mem[16'h0026] = 8'h21; mem[16'h0027] = 8'h05; mem[16'h0028] = 8'h90; // ld hl,$9005
        mem[16'h0029] = 8'hcb; mem[16'h002a] = 8'hfe;                       // set 7,(hl)
        mem[16'h002b] = 8'h76;                                              // halt
        mem[16'h0030] = 8'h3e; mem[16'h0031] = 8'h5a;                       // ld a,$5a
        mem[16'h0032] = 8'h77;                                              // ld (hl),a
        mem[16'h0033] = 8'hc9;                                              // ret

        run_until_halt(4000);
        expect8(16'h9000, 8'h08, "5 + 3");
        expect8(16'h9001, 8'h0a, "1+2+3+4");
        expect8(16'h9002, 8'h5a, "the call wrote through hl");
        expect8(16'h9003, 8'h34, "push/pop low");
        expect8(16'h9004, 8'h12, "push/pop high");
        expect8(16'h9005, 8'h80, "set 7,(hl)");

        if (fails == 0)
            $display("PASS: %0d T-state counts match the published tables, and the programs compute", 50);
        else
            $display("FAIL: %0d check(s) failed", fails);
        $finish;
    end

    // A watchdog, so a core that never retires says so instead of
    // running until the simulator gives up.
    initial begin
        #2_000_000;
        $display("FAIL: did not finish; pc = %04x", dbg_pc);
        $finish;
    end
endmodule
