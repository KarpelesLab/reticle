// timer — a prescaled down-counter with an auto-reload and an interrupt.
//
// What it does
//   A PRESCALE_WIDTH-bit prescaler divides `clk` by `prescale` + 1; every
//   prescaler tick decrements a WIDTH-bit counter. When the counter is at
//   zero and a tick arrives, `irq` goes high for one cycle and the
//   counter reloads from `reload`, so the period is
//
//       (reload + 1) * (prescale + 1)  cycles of clk
//
//   `value` is the counter, readable at any time. `irq_pending` is the
//   same event latched: it is set with `irq` and stays set until
//   `irq_clear` is high, which is what a processor polling a status
//   register needs. `load` reloads the counter and clears the prescaler
//   immediately, whatever `en` says.
//
//   `en` low freezes nothing: it holds the timer at `reload` with the
//   prescaler cleared, so enabling starts a full period rather than
//   whatever was left of one.
//
// What it does not do
//   One channel, down-counting, auto-reload always on: there is no
//   one-shot mode (hold `en` low from the interrupt handler instead), no
//   up-count or up/down mode, no capture input, no compare outputs and no
//   chaining of two timers into a wider one. `reload` and `prescale` are
//   sampled continuously rather than double buffered, so changing either
//   mid-period changes that period; pulse `load` after writing them if
//   that matters.
//
//   `irq` is a single-cycle pulse in the `clk` domain. Crossing it to
//   another domain needs `cdc_pulse`.
module timer #(
    // Counter bits.
    parameter WIDTH          = 16,
    // Prescaler bits.
    parameter PRESCALE_WIDTH = 8
) (
    input  wire                      clk,
    input  wire                      rst_n,

    input  wire                      en,
    // Reload the counter now, whatever `en` says.
    input  wire                      load,
    // Clock divisor minus one.
    input  wire [PRESCALE_WIDTH-1:0] prescale,
    // Counter start value; the period is (reload + 1) prescaler ticks.
    input  wire [WIDTH-1:0]          reload,

    output wire [WIDTH-1:0]          value,
    // One cycle high per expiry.
    output reg                       irq,
    // Sticky form of `irq`, cleared by `irq_clear`.
    output reg                       irq_pending,
    input  wire                      irq_clear
);
    reg [PRESCALE_WIDTH-1:0] pre_cnt;
    reg [WIDTH-1:0]          cnt;

    wire tick = (pre_cnt == prescale);

    assign value = cnt;

    always @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            pre_cnt     <= {PRESCALE_WIDTH{1'b0}};
            cnt         <= {WIDTH{1'b0}};
            irq         <= 1'b0;
            irq_pending <= 1'b0;
        end else begin
            irq <= 1'b0;
            if (irq_clear) irq_pending <= 1'b0;

            if (load || !en) begin
                pre_cnt <= {PRESCALE_WIDTH{1'b0}};
                cnt     <= reload;
            end else if (tick) begin
                pre_cnt <= {PRESCALE_WIDTH{1'b0}};
                if (cnt == {WIDTH{1'b0}}) begin
                    cnt         <= reload;
                    irq         <= 1'b1;
                    irq_pending <= 1'b1;
                end else begin
                    cnt <= cnt - 1'b1;
                end
            end else begin
                pre_cnt <= pre_cnt + 1'b1;
            end
        end
    end
endmodule
