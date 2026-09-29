// monitor_tb — a session at the monitor, written in Verilog.
//
// What it does
//   Waits for the prompt, types three lines, and prints the whole
//   transcript. `reticle sim` runs it, and so does
//   `tests/mos6502_monitor.rs`, which compares what comes out of
//   `$display` against the same transcript it computes for itself.
//
//   The typing is **handshaked and not timed**: `sendchar` waits for the
//   machine to echo the character before sending the next one, so the
//   run is the same length however CPU_DIV is set and a slow processor
//   cannot be typed over. That is the arrangement `examples/apple2`'s
//   testbench uses, for the same reason.
//
//   Carriage returns are turned into `|` in the transcript, because a
//   `$display` of a string with returns in it is one line on top of
//   another and nothing can be read off it.
module monitor_tb;
    localparam integer HALF = 8;             // 16 ns, near enough 60 MHz
    localparam integer LOGMAX = 256;

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

    // 115200, which is what a terminal program opens a port at and what
    // the 65C51's four baud bits cannot name — so `acia_control` should
    // come out with them clear, meaning "clocked from outside".
    reg [31:0] host_rate = 32'd115200;

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
    reg [7:0] log [0:LOGMAX-1];
    integer   printed = 0;
    always @(posedge clk) begin
        if (rst_n && print_valid && printed < LOGMAX) begin
            log[printed] = print_data;
            printed      = printed + 1;
        end
    end

    // One byte in, handshaked on the endpoint's own ready.
    task sendbyte(input [7:0] value);
        begin
            @(negedge clk);
            key_data  = value;
            key_valid = 1'b1;
            @(posedge clk);
            while (!key_ready) @(posedge clk);
            @(negedge clk);
            key_valid = 1'b0;
        end
    endtask

    // One character, then wait for the monitor to echo it. That is what
    // stops the testbench typing faster than a 6502 reads.
    task sendchar(input [7:0] value);
        integer mark;
        begin
            mark = printed;
            sendbyte(value);
            wait (printed != mark);
        end
    endtask

    task command(input [8*24-1:0] text, input integer n);
        integer        k;
        reg [8*24-1:0] rest;
        begin
            for (k = n; k > 0; k = k - 1) begin
                rest = text >> (8 * (k - 1));
                sendchar(rest[7:0]);
            end
            sendbyte(8'h0d);
        end
    endtask

    // Wait until the machine has printed `n` bytes in total, or give up.
    task settle(input integer n);
        integer guard;
        begin
            guard = 0;
            while (printed < n && guard < 2_000_000) begin
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

        // One byte, from the ROM's own first instruction.
        command("FF00", 4);
        settle(16);

        // A deposit, and reading it back as a range.
        command("0300: A9 41 60", 13);
        settle(33);
        command("0300.0303", 9);
        settle(59);

        // A line that is not a line: the monitor answers with a fresh
        // prompt and nothing else.
        command("HELLO", 5);
        settle(66);

        text = 0;
        for (i = 0; i < printed; i = i + 1) begin
            text = (text << 8) | ((log[i] == 8'h0d) ? 8'h7c : log[i]);
        end
        $display("%0s", text);
        $display("acia_control=%02x acia_rate=%0d", acia_control, acia_rate);
        $finish;
    end
endmodule
