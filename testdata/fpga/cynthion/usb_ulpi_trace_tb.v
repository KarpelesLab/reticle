// Does `usb_ulpi_trace.v` record and print the bus it says it does?
//
// The transceiver here is a stub: it answers register accesses, and then it
// delivers one SETUP token and one DATA0 packet, so the contents of the trace
// are known by hand and are asserted as one string.
//
// **The stub spaces a packet's bytes forty clocks apart**, with a receive
// command in between, because a full-speed line does — and because a stub that
// handed them over back to back hid a real bug: the trigger's second-byte match
// fired on the cycle *after* the PID instead of on the next byte of the packet,
// and on the part it therefore never fired at all.
//
// The other bug this asserts against is the rotation. The console walks the
// trace's shift register one entry per four nibbles and EVENTS of them bring it
// back where it started, so a dump that begins while the trace is still filling
// and rotates partway through leaves the register permanently out of step:
// every later dump then prints the right entries in the right cyclic order
// starting from the wrong one. The trace was four entries out until `dump_ok`
// was added, and this line is what says it is not.
module usb_ulpi_trace_tb;
    reg clk = 1'b0;
    reg        dir = 1'b0;
    reg        nxt = 1'b0;
    reg [7:0]  drv = 8'h00;
    wire [7:0] ulpi_data;
    wire ulpi_stp, ulpi_rst_n, ulpi_clk, uart_tx;
    wire l0, l1, l2, l3, l4, l5;

    assign ulpi_data = dir ? drv : 8'bz;

    usb_ulpi_trace #(
        .POR        (4),
        .BAUD_DIV   (9),
        .GAP_BITS   (3),
        .OPEN_BIT   (6),
        .SHUT_BIT   (17),
        .EVENTS     (8),
        .TRIG_PID   (8'h2D),
        .NEED_READY (1'b0)
    ) dut (
        .clk        (clk),
        .ulpi_data  (ulpi_data),
        .ulpi_dir   (dir),
        .ulpi_nxt   (nxt),
        .ulpi_stp   (ulpi_stp),
        .ulpi_rst_n (ulpi_rst_n),
        .ulpi_clk   (ulpi_clk),
        .uart_tx    (uart_tx),
        .led0_n     (l0),
        .led1_n     (l1),
        .led2_n     (l2),
        .led3_n     (l3),
        .led4_n     (l4),
        .led5_n     (l5)
    );

    always #1 clk = ~clk;

    // ---- the stub transceiver -------------------------------------------
    // A register write is command, `nxt`, byte, `nxt`, `stp`; a register read is
    // command, `nxt`, then `dir` for the turnaround and the byte. Nothing here
    // is throttled and nothing aborts. It never gets the link to `phy_ready`,
    // which is why the trigger's `NEED_READY` is off.
    reg [7:0] cmd = 8'h00;
    reg [2:0] st  = 3'd0;
    reg [7:0] regs_39 = 8'h04;
    reg [7:0] regs_04 = 8'h41;
    reg       traffic = 1'b0;

    always @(posedge clk) begin
        if (!traffic) begin
            case (st)
                3'd0: begin
                    nxt <= 1'b0;
                    dir <= 1'b0;
                    if (ulpi_rst_n && ulpi_data[7:6] != 2'b00
                                   && ulpi_data !== 8'hxx) begin
                        cmd <= ulpi_data;
                        nxt <= 1'b1;
                        st  <= 3'd1;
                    end
                end
                3'd1: begin
                    nxt <= 1'b0;
                    if (cmd[7:6] == 2'b10) begin
                        if (cmd[5:0] == 6'h39) regs_39 <= ulpi_data;
                        if (cmd[5:0] == 6'h04) regs_04 <= ulpi_data;
                        nxt <= 1'b1;
                        st  <= 3'd2;
                    end else begin
                        dir <= 1'b1;
                        drv <= 8'hAA;
                        st  <= 3'd3;
                    end
                end
                3'd2: begin
                    nxt <= 1'b0;
                    st  <= 3'd0;      // the link's `stp` ends it
                end
                3'd3: begin
                    drv <= (cmd[5:0] == 6'h39) ? regs_39
                         : (cmd[5:0] == 6'h04) ? regs_04
                         : (cmd[5:0] == 6'h15) ? 8'h01
                         :                       8'h00;
                    st  <= 3'd4;
                end
                default: begin
                    dir <= 1'b0;
                    drv <= 8'h00;
                    st  <= 3'd0;
                end
            endcase
        end
    end

    task packet(input [7:0] b0, input [7:0] b1, input [7:0] b2);
        begin
            @(posedge clk); dir <= 1'b1; nxt <= 1'b1; drv <= 8'hAA;
            @(posedge clk); drv <= b0;
            @(posedge clk); nxt <= 1'b0; drv <= 8'h59;
            repeat (8) @(posedge clk);
            @(posedge clk); nxt <= 1'b1; drv <= b1;
            @(posedge clk); nxt <= 1'b0; drv <= 8'h59;
            repeat (8) @(posedge clk);
            @(posedge clk); nxt <= 1'b1; drv <= b2;
            @(posedge clk); nxt <= 1'b0; drv <= 8'h01;
            @(posedge clk); dir <= 1'b0; drv <= 8'h00; nxt <= 1'b0;
            @(posedge clk);
        end
    endtask

    // ---- a receiver, and the line it should have received ----------------
    parameter integer BAUD  = 10;   // BAUD_DIV + 1 clocks a bit
    parameter integer CHARS = 37;   // 'X', four header nibbles, eight entries
    // The trace, in order: the token's third byte, the receive command that ends
    // it, the link idle, the link's own register read, the next packet's
    // turnaround, the DATA0's PID byte, a receive command, the byte after it.
    parameter [8*CHARS-1:0] WANT = "X300404598610040110F996AA06C304598680";

    integer   phase    = 0;
    integer   bitcount = 0;
    reg [7:0] got      = 8'h00;
    reg       rxing    = 1'b0;
    reg       prev     = 1'b1;
    integer   chars    = 0;
    reg [8*CHARS-1:0] line = {8*CHARS{1'b0}};
    reg [8*CHARS-1:0] last = {8*CHARS{1'b0}};
    reg       have     = 1'b0;

    always @(posedge clk) begin
        if (!rxing) begin
            if (prev === 1'b1 && uart_tx === 1'b0) begin
                rxing    <= 1'b1;
                phase    <= BAUD + BAUD / 2 - 1;
                bitcount <= 0;
            end
        end else if (phase == 0) begin
            got   <= {uart_tx, got[7:1]};
            phase <= BAUD - 1;
            if (bitcount == 7) begin
                rxing <= 1'b0;
                // The **last** whole line, because the first dumps go out while
                // the trace is still filling and say so in their flags.
                if ({uart_tx, got[7:1]} == 8'h58) begin
                    line  <= {{(8*CHARS-8){1'b0}}, 8'h58};
                    chars <= 1;
                end else if (chars > 0 && chars < CHARS) begin
                    line  <= (line << 8) | {{(8*CHARS-8){1'b0}}, {uart_tx, got[7:1]}};
                    chars <= chars + 1;
                    if (chars + 1 == CHARS) begin
                        last <= (line << 8)
                              | {{(8*CHARS-8){1'b0}}, {uart_tx, got[7:1]}};
                        have <= 1'b1;
                    end
                end
            end else begin
                bitcount <= bitcount + 1;
            end
        end else begin
            phase <= phase - 1;
        end
        prev <= uart_tx;
    end

    initial begin
        #4000;
        traffic = 1'b1;
        dir = 1'b0; nxt = 1'b0; drv = 8'h00;
        #10;
        packet(8'h2D, 8'h00, 8'h10);   // a SETUP token to address zero
        #20;
        packet(8'hC3, 8'h80, 8'h06);   // and its DATA0
        #20000;
        if (!have) begin
            $display("FAIL: the console never sent a whole line of %0d characters",
                     CHARS);
        end else if (last !== WANT) begin
            $display("FAIL: the trace reads %s", last);
            $display("      and should read  %s", WANT);
        end else begin
            $display("PASS: the trace is the bus, in order, from its oldest entry");
        end
        $finish;
    end
endmodule
