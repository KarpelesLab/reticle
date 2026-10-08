// A self-test for the 7-series backend, on a Digilent Basys 3, that
// reports its own result over the board's serial port.
//
// Every other design in this directory is checked by a person watching
// LEDs. That is a weak instrument for the things this backend has
// recently gained — a `CARRY4` chain, a `RAMB18E1` with contents, a
// `RAM64X1D` on a `SLICEM`, a `PLLE2_BASE` — because "LED 9 blinks at
// half the rate of LED 8" is an awkward thing to judge by eye and an
// impossible thing to put in a log. This design computes the answers
// instead and prints them, at 115200 baud on the board's USB-UART
// bridge, which the host sees as a serial port. The field layout is
// below, and decoding it needs nothing but `cut` and a calculator.
//
// **It has not reported anything yet**, and that is not this design's
// fault as far as anything here can tell: nothing this flow drives onto
// A18 has reached the host, including a bare counter bit with no UART in
// it (`docs/fpga-xray.md`, "What is now known about this part"). So the
// pin assignment is the open question, and this design is the instrument
// waiting on it.
//
// WHAT A PERSON SHOULD SEE: the four-digit display lit with a number,
// LD7 blinking about three times every two seconds, and LD0 to LD5 lit
// — LD5 is "every test passed". SW0, SW1 and SW2 choose which number
// the display shows (see THE DISPLAY). Nothing is required of the
// person at all, which is the point: the serial port carries the result.
//
// WHAT HAS BEEN TRIED ON A PART: see `docs/fpga-xray.md`. This header
// claims nothing; the document records what was measured and when.
//
// ===================================================================
// THE FOUR TESTS
// ===================================================================
//
// Each is chosen so that the ways of getting it wrong give different
// wrong answers, rather than all giving zero.
//
// 1. CARRY — sixteen additions of 32'hDEADBEEF into a 32-bit
//    accumulator. The expected value is 0xDEADBEEF * 16 truncated to
//    32 bits, which is **0xEADBEEF0**. It is a deliberately awkward
//    constant: bits change across every one of the eight `CARRY4`
//    boundaries during the sixteen steps, so a carry that fails to
//    cross any of them shows up, and the answer is not a round number
//    that a stuck-at fault could produce by accident.
//
// 2. BRAM — all 256 words of a block RAM whose word n holds n + 1,
//    read back and compared. Counts mismatches. This is `bram_rom.v`'s
//    test with the person taken out of it: contents, address order and
//    data order in one number that should be zero.
//
// 3. LUTRAM — sixteen words of four bits in a distributed RAM, written
//    at every address and then read back at every address. Word n
//    holds n ^ 4'hA, so all sixteen values differ and the test is
//    sensitive to an address bit landing in the wrong place.
//
//    **What this cannot catch**, and `lutram.v`'s header is right that
//    only a person could: a permutation applied *equally* to the write
//    and the read port. But that case is also not a fault — if both
//    ports agree, the memory is a correct sixteen-word RAM that has
//    merely relabelled its own rows, and nothing outside it can observe
//    the labels. What this does catch is the write port and the read
//    port **disagreeing**, which is the case that corrupts data, and
//    which is exactly what `src/fpga/xray/lutram.rs` says is taken from
//    nextpnr-xilinx and Project X-Ray's fuzzer rather than measured.
//
// 4. PLL — a 25 MHz clock asked for by frequency, counted against the
//    100 MHz oscillator over a window of 2^20 oscillator cycles. The
//    expected count is 2^20 / 4 = 2^18 = **0x40000**. The reported
//    number is the measurement, not a verdict: a PLL locked to the
//    wrong multiple gives a wrong count rather than no count, and the
//    count says which. `locked` is reported beside it.
//
//    A PLL that never starts leaves this test at zero and **does not
//    hang the others**: nothing outside the window counter is clocked
//    by it, and the sequencer that waits for the window is clocked by
//    the oscillator. That is deliberate. A self-test whose first
//    failure stops the report is a self-test that tells you one thing.
//
// ===================================================================
// WHAT IT SENDS
// ===================================================================
//
// Thirty-two hexadecimal characters and a CRLF, about three times every
// two seconds, forever. There is no framing, no checksum and no ASCII
// legend, because the host end is where complexity is cheap and the
// part is where it is expensive: a `case` statement spelling out
// "carry=" in characters is RTL that can be wrong, and every byte of it
// would have to be debugged through a serial port. One shift register
// and a nibble-to-hex function cannot be subtly wrong — it either
// produces hex or produces nothing.
//
//   [127:96]  carry accumulator          expect EADBEEF0
//   [ 95:64]  PLL cycles in the window   expect 00040000
//   [ 63:48]  block RAM mismatches       expect 0000
//   [ 47:32]  distributed RAM mismatches expect 0000
//   [ 31:16]  a fixed word, A5C3         expect A5C3
//   [ 15: 8]  status bits (below)
//   [  7: 0]  report sequence number, which wraps
//
// `A5C3` is the one field with no test behind it, and it earns its
// place: it is the evidence that the baud rate, the bit order and the
// character framing are right. Without it, a UART sending plausible
// rubbish and a part computing rubbish look the same from the host.
//
// Status bits, least significant first: carry ok, block RAM ok,
// distributed RAM ok, PLL count ok, PLL locked. The top three are zero,
// so a passing part reports status 1F.
//
// The sequence number is what distinguishes a part that is running from
// a part that printed one line and stopped, which a static capture
// cannot otherwise show.
//
// ===================================================================
// THE DISPLAY
// ===================================================================
//
// SW2, SW1, SW0 read as a binary number choose which sixteen bits of
// the report the four digits show, in hexadecimal:
//
//   0  status and sequence number   4  carry accumulator, high half
//   1  block RAM mismatches         5  PLL cycles, low half
//   2  distributed RAM mismatches   6  PLL cycles, high half
//   3  carry accumulator, low half  7  the constant 0123
//
// Mode 7 exists to settle something this repository does not know. The
// Basys 3's four digit anodes are `an[0]` on ball U2 through `an[3]` on
// W4, and **which end of the display `an[0]` is at has not been
// verified here**. Mode 7 drives the fixed word 0x0123 with digit 0
// holding the 3, so whoever is at the board can read the answer off the
// glass: "0123" means `an[0]` is the right-hand digit, "3210" means it
// is the left-hand one. Until that is known every other mode is still
// readable — the digits are either in the order written here or
// reversed — and no test depends on it.
//
// Segments and anodes are both active low (the display is common anode,
// switched by PNP transistors), which is Digilent's reference manual and
// not a measurement. The decimal point is held off.

module selftest #(
    // Clock cycles per bit on the serial line. 100 MHz / 115200 = 868.
    parameter CLK_DIV = 868,
    // Which bit of a free-running counter paces the report: bit 25 of a
    // 100 MHz counter changes every 0.336 s.
    parameter TICK_BIT = 25,
    // The PLL measurement window, as a power of two oscillator cycles.
    // The expected count is a quarter of it, the 25 MHz clock being a
    // quarter of the 100 MHz one.
    parameter WINDOW_BITS = 20
) (
    input  wire       clk,
    // BTNC, which restarts every test. Active high, no external pull.
    input  wire       btn,
    // SW0, SW1, SW2: which field the display shows.
    input  wire [2:0] sw,

    // One LED per test, then "all of them", then a heartbeat. LD6 (U14)
    // is skipped: it is the one pin of a `LIOB33_SING` tile that this
    // flow has no table for, which `lutram.rcf` found first.
    output wire       led_carry,
    output wire       led_bram,
    output wire       led_lut,
    output wire       led_pll,
    output wire       led_locked,
    output wire       led_all,
    output wire       led_alive,

    // The four-digit display: seven segments, the point, four anodes.
    output wire [6:0] seg,
    output wire       dp,
    output wire [3:0] an,

    output wire       uart_tx_pin
);
    // ---- The 25 MHz clock, asked for by frequency. ----
    //
    // A clock net nothing drives, with a frequency on it, is how a
    // design asks for a PLL; a net nothing drives that names that clock
    // is its LOCKED. `pll_blink.v` is the smaller example of both.
    (* clock_mhz = 25 *) wire clk_pll;
    (* clock_locked = "clk_pll" *) wire locked;

    // ---- Reset: the button, through two flip-flops. ----
    //
    // The board has no reset pin. The design resets itself when the part
    // is configured, because every register below has an initial value,
    // and BTNC runs the tests again.
    reg [1:0] btn_sync = 2'b00;
    always @(posedge clk) btn_sync <= {btn_sync[0], btn};
    wire restart = btn_sync[1];

    // And a power-on reset, which the serial transmitter needs even
    // though nothing else here does.
    //
    // Every register in this file has an initial value, and on this part
    // an initial value is a real thing: the bitstream carries it, so the
    // design starts where it says it starts. `ip/bus/uart` does not work
    // that way — it is written for any fabric and resets on `rst_n`, so
    // tying that high leaves its registers undefined. In simulation that
    // showed as `tx` stuck at X forever, which is how this was found;
    // on the part it would have been a transmitter that came up in
    // whatever state the bitstream's defaults left it in, and
    // `drive_constant_data` is a reminder of how badly that can read.
    reg [3:0] por = 4'd0;
    always @(posedge clk) if (por != 4'hF) por <= por + 4'd1;
    wire rst_n = (por == 4'hF) & ~restart;

    // =================================================================
    // The test sequencer
    // =================================================================

    localparam S_CARRY  = 3'd0,
               S_BRAM   = 3'd1,
               S_LUTW   = 3'd2,
               S_LUTR   = 3'd3,
               S_PLL    = 3'd4,
               S_SETTLE = 3'd5,
               S_READY  = 3'd6;

    reg [2:0] state = S_CARRY;

    // ---- 1. The carry chain. ----
    reg [31:0] acc   = 32'd0;
    reg [4:0]  steps = 5'd0;

    // ---- 2. The block RAM. ----
    (* ram_style = "block" *)
    reg [7:0] rom [0:255];
    integer i;
    initial begin
        for (i = 0; i < 256; i = i + 1) rom[i] = i[7:0] + 8'd1;
    end
    reg [7:0]  rom_addr  = 8'd0;
    reg [7:0]  rom_q     = 8'd0;
    // The address the word now in `rom_q` was read from. A block RAM's
    // output is a register, so the comparison is one cycle behind the
    // address, and comparing against the current address would pass a
    // memory that returned the previous word.
    reg [7:0]  rom_shown = 8'd0;
    reg        rom_valid = 1'b0;
    reg [15:0] bram_bad  = 16'd0;

    always @(posedge clk) begin
        rom_q     <= rom[rom_addr];
        rom_shown <= rom_addr;
    end

    // ---- 3. The distributed RAM. ----
    reg  [3:0] mem [0:15];
    reg  [3:0] lut_addr = 4'd0;
    reg [15:0] lut_bad  = 16'd0;
    wire [3:0] lut_q    = mem[lut_addr];

    // ---- 4. The PLL. ----
    reg [WINDOW_BITS-1:0] window = {WINDOW_BITS{1'b0}};
    wire                  counting = (state == S_PLL);
    // In the PLL's own domain: the window brought across, and the count.
    // The counter is cleared when the window opens rather than at reset,
    // so a re-run measures a fresh window; three synchroniser stages
    // make "the window just opened" an edge rather than a level, and
    // cost at most two counts out of 262144, which is below the
    // resolution this measurement needs.
    reg [2:0]             win_sync = 3'b000;
    reg [WINDOW_BITS-1:0] pll_cnt  = {WINDOW_BITS{1'b0}};
    always @(posedge clk_pll) begin
        win_sync <= {win_sync[1:0], counting};
        if (win_sync[1] & ~win_sync[2]) pll_cnt <= {WINDOW_BITS{1'b0}};
        else if (win_sync[2])           pll_cnt <= pll_cnt + 1'b1;
    end
    // And back, for the report. `S_SETTLE` is long enough that `pll_s2`
    // has the final count before it is latched.
    reg [WINDOW_BITS-1:0] pll_s1 = {WINDOW_BITS{1'b0}},
                          pll_s2 = {WINDOW_BITS{1'b0}};
    reg [31:0]            pll_count = 32'd0;
    reg [3:0]             settle = 4'd0;
    always @(posedge clk) begin
        pll_s1 <= pll_cnt;
        pll_s2 <= pll_s1;
    end

    always @(posedge clk) begin
        if (restart) begin
            state     <= S_CARRY;
            acc       <= 32'd0;
            steps     <= 5'd0;
            rom_addr  <= 8'd0;
            rom_valid <= 1'b0;
            bram_bad  <= 16'd0;
            lut_addr  <= 4'd0;
            lut_bad   <= 16'd0;
            window    <= {WINDOW_BITS{1'b0}};
            settle    <= 4'd0;
            pll_count <= 32'd0;
        end else begin
            case (state)
                S_CARRY: begin
                    acc   <= acc + 32'hDEADBEEF;
                    steps <= steps + 5'd1;
                    if (steps == 5'd15) state <= S_BRAM;
                end
                S_BRAM: begin
                    // `rom_q` holds the word read from `rom_shown`, one
                    // cycle behind. `rom_valid` skips the first cycle,
                    // when nothing has been read yet.
                    if (rom_valid && rom_q != (rom_shown + 8'd1))
                        bram_bad <= bram_bad + 16'd1;
                    rom_valid <= 1'b1;
                    if (rom_valid && rom_shown == 8'd255) begin
                        state     <= S_LUTW;
                        rom_valid <= 1'b0;
                    end else begin
                        rom_addr <= rom_addr + 8'd1;
                    end
                end
                S_LUTW: begin
                    mem[lut_addr] <= lut_addr ^ 4'hA;
                    if (lut_addr == 4'd15) begin
                        state    <= S_LUTR;
                        lut_addr <= 4'd0;
                    end else begin
                        lut_addr <= lut_addr + 4'd1;
                    end
                end
                S_LUTR: begin
                    // Read without a clock: `lut_q` is the word at
                    // `lut_addr` in this same cycle.
                    if (lut_q != (lut_addr ^ 4'hA))
                        lut_bad <= lut_bad + 16'd1;
                    if (lut_addr == 4'd15) state <= S_PLL;
                    else lut_addr <= lut_addr + 4'd1;
                end
                S_PLL: begin
                    window <= window + 1'b1;
                    if (window == {WINDOW_BITS{1'b1}}) begin
                        state  <= S_SETTLE;
                        settle <= 4'd0;
                    end
                end
                S_SETTLE: begin
                    settle <= settle + 4'd1;
                    if (settle == 4'd15) begin
                        pll_count <= {{(32 - WINDOW_BITS){1'b0}}, pll_s2};
                        state     <= S_READY;
                    end
                end
                default: begin
                    // Nothing left to do: the report repeats.
                    state <= S_READY;
                end
            endcase
        end
    end

    // =================================================================
    // The verdicts
    // =================================================================

    // Every verdict requires `ready`, and that is not decoration. A
    // mismatch counter sitting at zero is vacuously clean: before the
    // block RAM test has read anything, "no mismatches" is true and
    // means nothing. Without this the LEDs claimed three passes within
    // a microsecond of configuration, which is what the first
    // simulation of this design showed.
    wire ready    = (state == S_READY);
    wire carry_ok = ready & (acc == 32'hEADBEEF0);
    wire bram_ok  = ready & (bram_bad == 16'd0);
    wire lut_ok   = ready & (lut_bad == 16'd0);
    // A quarter of the window: the 25 MHz clock against the 100 MHz one.
    wire pll_ok   = ready & (pll_count == (32'd1 << (WINDOW_BITS - 2)));
    wire all_ok   = carry_ok & bram_ok & lut_ok & pll_ok & locked;

    reg  [7:0] seq = 8'd0;
    wire [7:0] status = {3'b000, locked, pll_ok, lut_ok, bram_ok, carry_ok};

    wire [127:0] report = {
        acc,                    // [127:96]
        pll_count,              // [ 95:64]
        bram_bad,               // [ 63:48]
        lut_bad,                // [ 47:32]
        16'hA5C3,               // [ 31:16]
        status,                 // [ 15: 8]
        seq                     // [  7: 0]
    };

    // =================================================================
    // Sending it
    // =================================================================

    reg [TICK_BIT:0] tick   = {(TICK_BIT + 1){1'b0}};
    reg              tick_d = 1'b0;
    always @(posedge clk) begin
        tick   <= tick + 1'b1;
        tick_d <= tick[TICK_BIT];
    end
    wire tick_edge = tick[TICK_BIT] & ~tick_d;

    // The emitter. `left` counts the hex characters still to send and
    // `tail` the CR and LF after them, so between them they are the
    // position in the line and no separate index is needed.
    //
    // The handshake is presented and then held: `tx_valid` goes high
    // with a character and stays high until `tx_ready` is high in the
    // same cycle, which is when the character has been taken. Pulsing
    // `tx_valid` for one cycle instead would be a race — the cycle it
    // appears in is not the cycle its `tx_ready` was read in.
    reg [127:0] shifter  = 128'd0;
    reg [5:0]   left     = 6'd0;
    reg [1:0]   tail     = 2'd0;
    reg         sending  = 1'b0;
    reg [7:0]   tx_data  = 8'd0;
    reg         tx_valid = 1'b0;
    wire        tx_ready;

    function [7:0] hex;
        input [3:0] nibble;
        hex = (nibble < 4'd10) ? (8'd48 + {4'd0, nibble})   // '0'
                               : (8'd55 + {4'd0, nibble});  // 'A'
    endfunction

    always @(posedge clk) begin
        if (!sending) begin
            if (tick_edge && ready) begin
                shifter <= report;
                left    <= 6'd32;
                tail    <= 2'd2;
                sending <= 1'b1;
            end
        end else if (!tx_valid) begin
            // Present the character at the current position.
            if (left != 6'd0)        tx_data <= hex(shifter[127:124]);
            else if (tail == 2'd2)   tx_data <= 8'd13;
            else                     tx_data <= 8'd10;
            tx_valid <= 1'b1;
        end else if (tx_ready) begin
            // It has been taken; advance.
            tx_valid <= 1'b0;
            if (left != 6'd0) begin
                shifter <= {shifter[123:0], 4'd0};
                left    <= left - 6'd1;
            end else if (tail == 2'd2) begin
                tail <= 2'd1;
            end else begin
                tail    <= 2'd0;
                sending <= 1'b0;
                seq     <= seq + 8'd1;
            end
        end
    end

    // 100 MHz / 115200 = 868.
    uart_tx #(
        .CLK_DIV(CLK_DIV)
    ) tx_half (
        .clk      (clk),
        .rst_n    (rst_n),
        .div      (16'd0),
        .tx_data  (tx_data),
        .tx_valid (tx_valid),
        .tx_ready (tx_ready),
        .tx       (uart_tx_pin)
    );

    // =================================================================
    // The display
    // =================================================================

    wire [15:0] shown =
        (sw == 3'd0) ? {status, seq}              :
        (sw == 3'd1) ? bram_bad                   :
        (sw == 3'd2) ? lut_bad                    :
        (sw == 3'd3) ? acc[15:0]                  :
        (sw == 3'd4) ? acc[31:16]                 :
        (sw == 3'd5) ? pll_count[15:0]            :
        (sw == 3'd6) ? pll_count[31:16]           :
                       16'h0123;

    // One digit at a time, each for 2^16 cycles: 381 Hz per digit, and
    // 1526 Hz around the four — well above the eye and well below the
    // rate at which the anode transistors would smear one into the next.
    reg [17:0] mux = 18'd0;
    always @(posedge clk) mux <= mux + 18'd1;
    wire [1:0] digit = mux[17:16];

    wire [3:0] nibble =
        (digit == 2'd0) ? shown[3:0]   :
        (digit == 2'd1) ? shown[7:4]   :
        (digit == 2'd2) ? shown[11:8]  :
                          shown[15:12];

    // Active low, segments a..g in that order.
    function [6:0] segments;
        input [3:0] value;
        case (value)
            4'h0: segments = 7'b0000001;
            4'h1: segments = 7'b1001111;
            4'h2: segments = 7'b0010010;
            4'h3: segments = 7'b0000110;
            4'h4: segments = 7'b1001100;
            4'h5: segments = 7'b0100100;
            4'h6: segments = 7'b0100000;
            4'h7: segments = 7'b0001111;
            4'h8: segments = 7'b0000000;
            4'h9: segments = 7'b0000100;
            4'hA: segments = 7'b0001000;
            4'hB: segments = 7'b1100000;
            4'hC: segments = 7'b0110001;
            4'hD: segments = 7'b1000010;
            4'hE: segments = 7'b0110000;
            default: segments = 7'b0111000;
        endcase
    endfunction

    assign seg = segments(nibble);
    assign dp  = 1'b1;
    assign an  = ~(4'd1 << digit);

    assign led_carry  = carry_ok;
    assign led_bram   = bram_ok;
    assign led_lut    = lut_ok;
    assign led_pll    = pll_ok;
    assign led_locked = locked;
    assign led_all    = all_ok;
    assign led_alive  = tick[TICK_BIT];
endmodule
