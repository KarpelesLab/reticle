// typedef, enum and a state register that uses them.
module sv_typedef(
    input  logic clk,
    input  logic rst_n,
    input  logic go,
    output logic busy
);
    typedef enum logic [1:0] {
        IDLE = 2'b00,
        RUN  = 2'b01,
        DONE = 2'b10
    } state_t;

    typedef logic [7:0] byte_t;

    state_t state;
    byte_t  counter;

    always_ff @(posedge clk or negedge rst_n) begin
        if (!rst_n) begin
            state   <= IDLE;
            counter <= '0;
        end else begin
            case (state)
                IDLE: if (go) state <= RUN;
                RUN:  begin
                    counter <= counter + 1'b1;
                    if (counter == 8'hff) state <= DONE;
                end
                default: state <= IDLE;
            endcase
        end
    end

    assign busy = (state == RUN);
endmodule
