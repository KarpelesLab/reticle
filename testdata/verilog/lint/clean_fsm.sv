// A Moore FSM with an enum state type, always_ff / always_comb and a
// unique case. Taken from the parser corpus: a real design must lint
// silently.
module traffic_light (
    input  logic clk,
    input  logic rst_n,
    input  logic car_waiting,
    output logic red,
    output logic yellow,
    output logic green
);
    typedef enum logic [1:0] {
        S_RED    = 2'b00,
        S_GREEN  = 2'b01,
        S_YELLOW = 2'b10
    } state_t;

    state_t state, next_state;
    logic [3:0] timer;
    logic       timer_done;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) state <= S_RED;
        else        state <= next_state;
    end

    always_ff @(posedge clk) begin
        if (state != next_state) timer <= '0;
        else                     timer <= timer + 1'b1;
    end

    assign timer_done = &timer;

    always_comb begin
        next_state = state;
        unique case (state)
            S_RED:    if (timer_done && car_waiting) next_state = S_GREEN;
            S_GREEN:  if (timer_done)                next_state = S_YELLOW;
            S_YELLOW: if (timer[1])                  next_state = S_RED;
        endcase
    end

    always_comb begin
        {red, yellow, green} = 3'b000;
        priority case (state)
            S_RED:    red    = 1'b1;
            S_GREEN:  green  = 1'b1;
            S_YELLOW: yellow = 1'b1;
            default:  red    = 1'b1;
        endcase
    end
endmodule
