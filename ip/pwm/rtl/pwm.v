// pwm — a counter-comparator pulse width modulator.
//
// What it does
//   A free-running WIDTH-bit counter wraps every 2**WIDTH cycles of
//   `clk`; `pwm_out` is high while the counter is below the duty value,
//   so the output is high for `duty` cycles out of every 2**WIDTH. At
//   12 MHz with WIDTH = 8 that is a 46.9 kHz carrier with 256 steps.
//
//   `duty` is sampled once per period, at the wrap, into `duty_q`. That
//   is what stops a duty value changing mid-period from producing a pulse
//   of some third width — a glitch a motor or an LED would see as a
//   flicker. The value in use is on `duty_active`, so a register file can
//   report what is actually being driven rather than what was asked for.
//
//   `period_tick` is high for the one cycle that ends a period, which is
//   where a caller can update `duty` or count periods.
//
//   `en` low stops the counter, holds `pwm_out` low and keeps `duty_q`
//   loaded from `duty`, so the first period after enabling already uses
//   the requested duty.
//
// What it does not do
//   The duty is out of 2**WIDTH, so `duty` = 0 is a constant low and the
//   largest value, 2**WIDTH - 1, is one cycle short of a constant high:
//   there is no 100% setting. That is the price of a WIDTH-bit duty for a
//   2**WIDTH-cycle period, and the usual fix is to force the output high
//   outside this block when the caller means "full on".
//
//   The period is a power of two and nothing else: no programmable top,
//   no prescaler (put `timer` in front, or divide the clock), no
//   centre-aligned or phase-correct mode, no dead-time insertion and no
//   complementary output, so it must not be used to drive both sides of a
//   half bridge. One channel per instance.
module pwm #(
    // Counter bits; the period is 2**WIDTH cycles.
    parameter WIDTH = 8
) (
    input  wire             clk,
    input  wire             rst_n,
    input  wire             en,

    // Cycles high per period, out of 2**WIDTH.
    input  wire [WIDTH-1:0] duty,

    output wire             pwm_out,
    output wire [WIDTH-1:0] duty_active,
    output wire             period_tick
);
    reg [WIDTH-1:0] cnt;
    reg [WIDTH-1:0] duty_q;

    assign pwm_out     = en && (cnt < duty_q);
    assign duty_active = duty_q;
    assign period_tick = en && (cnt == {WIDTH{1'b1}});

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            cnt    <= {WIDTH{1'b0}};
            duty_q <= {WIDTH{1'b0}};
        end else if (!en) begin
            cnt    <= {WIDTH{1'b0}};
            duty_q <= duty;
        end else begin
            cnt <= cnt + 1'b1;
            if (cnt == {WIDTH{1'b1}}) duty_q <= duty;
        end
    end
endmodule
