// monitor_bench — the machine with its byte interface brought out, and
// nothing else.
//
// What it is for
//   `tests/mos6502_monitor.rs` types at the monitor and reads what it
//   prints, a character at a time, from Rust — so a command's answer can
//   be compared against a string the test computed rather than against a
//   string a testbench was told to expect. That needs the stream on the
//   outside of a module and the clock in the test's hands, which is all
//   this file is.
//
//   `tb/monitor_tb.v` is the other shape: a session written in Verilog,
//   for `reticle sim` and for anyone who wants to watch one run.
//
//   CPU_DIV is **1** here and 59 in the design. The processor's speed is
//   a divider and nothing in the machine depends on which value it has —
//   the bus is the same bus at one clock a cycle as at fifty-nine — so a
//   simulation that spends fifty-nine clocks on every cycle would be
//   fifty-nine times longer and would prove exactly the same thing. What
//   it would additionally prove is that the divider counts, and
//   `the_processor_runs_at_one_cycle_in_fifty_nine` is that, separately
//   and cheaply.
module monitor_bench #(
    parameter integer CPU_DIV    = 1,
    parameter integer RAM_BYTES  = 4096,
    // Short, because a test that waits 273 us of simulated time for a
    // packet boundary is a test that waits. What `in_commit` means is
    // checked by `a_partly_filled_packet_goes_when_the_machine_falls_silent`,
    // which sets it back to something board-like.
    parameter integer FLUSH_CLKS = 64
) (
    input  wire        clk,
    input  wire        rst_n,

    // Typing at it.
    input  wire [7:0]  key_data,
    input  wire        key_valid,
    output wire        key_ready,

    // And reading what it prints.
    output wire [7:0]  print_data,
    output wire        print_valid,
    input  wire        print_ready,
    output wire        print_commit,

    input  wire [31:0] host_rate,
    output wire [31:0] acia_rate,
    output wire [7:0]  acia_control
);
    monitor_machine #(
        .CPU_DIV    (CPU_DIV),
        .RAM_BYTES  (RAM_BYTES),
        .FLUSH_CLKS (FLUSH_CLKS)
    ) dut (
        .clk          (clk),
        .rst_n        (rst_n),
        .out_data     (key_data),
        .out_valid    (key_valid),
        .out_ready    (key_ready),
        .in_data      (print_data),
        .in_valid     (print_valid),
        .in_ready     (print_ready),
        .in_commit    (print_commit),
        .host_rate    (host_rate),
        .acia_rate    (acia_rate),
        .acia_control (acia_control),
        .print_data   (),
        .print_valid  ()
    );
endmodule
