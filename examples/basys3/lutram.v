// A sixteen-word, four-bit distributed RAM on a Digilent Basys 3, written
// from the switches and read back onto the LEDs.
//
// This is the hardware test for the one thing about a 7-series distributed
// RAM that nothing on this machine can check: **which lookup-table input
// is which address bit**, on the write port and on the read port. The
// flow puts each bit of this memory in one `RAM64X1D`, on one `SLICEM`,
// with the write address on the `D` lookup table's inputs and the read
// address on the `C` lookup table's, both in order. That reading is
// nextpnr-xilinx's and Project X-Ray's fuzzer's, not a measurement (see
// `src/fpga/xray/lutram.rs`). If it were wrong the design would place,
// route, decode bit for bit, configure with `DONE` high — and show a word
// written at one address when a different address is read. Only a person
// at the board can see that.
//
// THE CONTROLS (labels as printed on the board):
//
//   SW0..SW3    the write address, SW0 its lowest bit
//   SW4..SW7    the word to write, SW4 its lowest bit
//   SW8..SW11   the read address, SW8 its lowest bit
//   BTNC        write: while it is held, the word on SW4..SW7 is written
//               at the address on SW0..SW3, once every clock
//   LD0..LD3    the word stored at the read address, LD0 its lowest bit
//   LD5         lit while the write is happening
//   LD7         blinks about three times every two seconds: the design is
//               loaded and the clock runs
//
// WHAT A PERSON SHOULD DO AND SEE:
//
//   1. Load it with every switch down. LD7 blinks, LD0..LD3 are dark:
//      a distributed RAM with no initial contents powers up holding zero,
//      at every address.
//   2. For each k of 0, 1, 2, 3 in turn: put up ONLY switch SW<k> (write
//      address 1<<k) and switch SW<4+k> (the word 1<<k), press and release
//      BTNC, then put every switch down again. That stores the word
//      1, 2, 4, 8 at the addresses 1, 2, 4, 8.
//   3. Now read them back on SW8..SW11 alone: SW8 up should light LD0
//      only, SW9 alone LD1 only, SW10 alone LD2 only, SW11 alone LD3 only.
//      **That is the permutation test.** If the write and read ports
//      disagreed about which input is which address bit, address 1<<k
//      would show some other word or nothing.
//   4. Any other read address — all down (0), SW8+SW9 (3), SW8+SW10 (5),
//      all four up (15) — should show all LEDs dark, because nothing was
//      written there.
//   5. Write 1111 at address 15 (SW0..SW7 all up, BTNC), switches down,
//      then SW8..SW11 all up: LD0..LD3 all lit. Addresses 1, 2, 4, 8 still
//      show what step 2 put there.
//
// Pressing BTNC while switches are moving writes whatever the switches say
// at that moment, so set the switches first and press second.
//
// # Why there is no debounce
//
// A bouncing button here only writes the same word at the same address
// several times over, which leaves the memory exactly as one write would.
// So the button goes through two flip-flops, which keeps a change on it
// from arriving in the middle of a clock edge, and nothing more. A
// debounce counter would be four bits of `+` or more, and a 7-series carry
// chain does not route on this flow yet.
//
// # The heartbeat
//
// Is `examples/basys3/blink.v`'s counter, unchanged and for the same
// reason: its increment is spelled out as a toggle chain because a
// `count + 1` would need a carry chain. It is there so that a dark row of
// LEDs in step 1 means "the memory reads zero" rather than "nothing is
// loaded".
module lutram (
    input  wire       clk,
    input  wire       btn,
    input  wire [3:0] waddr,
    input  wire [3:0] wdata,
    input  wire [3:0] raddr,
    output wire [3:0] rdata,
    output wire       wrote,
    output wire       alive
);
    // The button, brought into the clock domain.
    reg [1:0] sync = 2'b00;
    always @(posedge clk) begin
        sync <= {sync[0], btn};
    end
    wire we = sync[1];

    // Sixteen words of four bits, written on the clock and read without
    // one: below the block RAM threshold, so it becomes four `RAM64X1D`.
    reg [3:0] mem [0:15];
    always @(posedge clk) begin
        if (we)
            mem[waddr] <= wdata;
    end
    assign rdata = mem[raddr];
    assign wrote = we;

    // The heartbeat: bit 25 of a counter off the 100 MHz oscillator.
    reg [25:0] count = 26'd0;
    wire [25:0] toggle;
    assign toggle[0] = 1'b1;
    genvar i;
    generate
        for (i = 1; i < 26; i = i + 1) begin : carry
            assign toggle[i] = &count[i-1:0];
        end
    endgenerate
    always @(posedge clk) begin
        count <= count ^ toggle;
    end
    assign alive = count[25];
endmodule
