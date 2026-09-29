// monitor_tb — a session at the monitor, written in Verilog.
//
// What it does
//   Waits for the prompt, types three lines, and prints the whole
//   transcript. `reticle sim` runs it, and so does
//   `tests/mos6502_monitor.rs`, which checks what comes out of
//   `$display` against what the monitor should have said.
//
//   The typing is **handshaked and not timed**: `sendbyte` waits for the
//   ACIA to have room, which it has only once the monitor has read the
//   last character, so the run is the same length however CPU_DIV is set
//   and a slow processor cannot be typed over. That is the arrangement
//   `examples/apple2`'s testbench uses, for the same reason.
//
//   Every address it examines is in the **ROM**, on purpose. A
//   distributed RAM cannot be given initial contents, so in simulation
//   every byte of the machine's RAM is `x` until something writes it,
//   and a transcript with `x` in it says nothing.
//   `tests/mos6502_monitor.rs` fills the RAM from outside before reset
//   and then uses it; a Verilog testbench has no clean way to, so this
//   one does not try.
//
//   Carriage returns are turned into `|` in the transcript, because a
//   `$display` of a string with returns in it is one line on top of
//   another and nothing can be read off it.
module monitor_tb;
    localparam integer HALF   = 8;      // 16 ns, near enough 60 MHz
    localparam integer LOGMAX = 96;

    reg clk = 1'b0;
    always #HALF clk = ~clk;

    reg rst_n = 1'b0;

    reg  [7:0]  key_data  = 8'd0;
    reg         key_valid = 1'b0;
    wire        key_ready;
    wire [7:0]  print_data;
    wire        print_valid;
    wire [31:0] acia_rate;
    wire [7:0]  acia_control;

    // What `usb_cdc_req` reports before any host has spoken. The monitor
    // programmes CONTROL a few hundred cycles after reset and wins;
    // `newrate` below is a host opening the port afterwards, which is
    // the order it happens in.
    reg [31:0] host_rate = 32'd9600;

    monitor_machine #(
        .CPU_DIV    (1),
        .RAM_BYTES  (4096),
        .FLUSH_CLKS (64)
    ) dut (
        .clk          (clk),
        .rst_n        (rst_n),
        .out_data     (key_data),
        .out_valid    (key_valid),
        .out_ready    (key_ready),
        .in_data      (print_data),
        .in_valid     (print_valid),
        .in_ready     (1'b1),
        .in_commit    (),
        .host_rate    (host_rate),
        .acia_rate    (acia_rate),
        .acia_control (acia_control),
        .print_data   (),
        .print_valid  ()
    );

    // Everything the machine has printed.
    reg [7:0]  log [0:LOGMAX-1];
    reg [31:0] printed = 32'd0;
    always @(posedge clk) begin
        if (rst_n && print_valid && printed < LOGMAX) begin
            log[printed] <= print_data;
            printed      <= printed + 32'd1;
        end
    end

    // One byte in, once the ACIA has room for it — which it has only
    // when the monitor has taken the last one.
    task sendbyte(input [7:0] value);
        begin
            @(negedge clk);
            while (!key_ready) @(negedge clk);
            key_data  = value;
            key_valid = 1'b1;
            @(negedge clk);
            key_valid = 1'b0;
        end
    endtask

    task command(input [8*16-1:0] text, input integer n);
        integer        k;
        reg [8*16-1:0] rest;
        begin
            for (k = n; k > 0; k = k - 1) begin
                rest = text >> (8 * (k - 1));
                sendbyte(rest[7:0]);
            end
            sendbyte(8'h0d);
        end
    endtask

    // Wait until the machine has printed `n` bytes in all, or give up.
    task settle(input integer n);
        integer guard;
        begin
            guard = 0;
            while (printed < n && guard < 2000000) begin
                @(posedge clk);
                guard = guard + 1;
            end
            if (printed < n) begin
                $display("monitor_tb: only %0d byte(s) printed, wanted %0d", printed, n);
                $finish;
            end
        end
    endtask

    integer i;
    reg [8*LOGMAX-1:0] text;

    initial begin
        repeat (8) @(posedge clk);
        rst_n = 1'b1;

        // The prompt: a backslash and a carriage return.
        settle(2);

        // One byte, the ROM's own first instruction. Seventeen: five
        // echoed, the monitor's own return, eight of answer, and the
        // return that begins the next line.
        command("FE00", 4);
        settle(17);

        // The three vectors, which are six bytes on one row because a
        // row breaks where the low three address bits are clear and
        // $FFF8 is the last of those. Ten echoed, a return, `FFFA:`,
        // six times three, and a return: thirty-five more.
        command("FFFA.FFFF", 9);
        settle(52);

        // A host opens the port at 115200, which the 65C51's four baud
        // bits cannot name — so the ACIA should report the whole external
        // configuration, `$00`: no generator selected and no rate, which
        // is what a bit clock arriving from outside the part is.
        host_rate = 32'd115200;
        repeat (200) @(posedge clk);

        // A line that is not a line: six echoed, a backslash and a
        // return, and nothing else.
        command("HELLO", 5);
        settle(60);

        text = 0;
        for (i = 0; i < printed; i = i + 1) begin
            text = (text << 8) | ((log[i] == 8'h0d) ? 8'h7c : log[i]);
        end
        $display("%0s", text);
        $display("acia_control=%02x acia_rate=%0d", acia_control, acia_rate);
        $finish;
    end
endmodule
