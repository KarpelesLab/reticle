package pkg;
    typedef enum logic [1:0] { IDLE, RUN, DONE } state_t;
    typedef struct packed { logic [7:0] a; bit b; } pair_t;
    typedef union packed { logic [7:0] a; byte b; } u_t;
    localparam int N = 4;
    function automatic int f(input int x);
        return x + 1;
    endfunction
endpackage

interface bus_if(input logic clk);
    logic req, ack;
    modport master(output req, input ack);
    modport slave(input req, output ack);
endinterface

module top import pkg::*; (input logic clk, rst, bus_if.master m);
    state_t state;
    longint unsigned big;
    shortint s; shortreal sr; real r; time t; realtime rt; chandle h;
    var logic v; const int c = 1; static int st; string str;

    always_ff @(posedge clk or posedge rst)
        if (rst) state <= IDLE;
        else unique case (state)
            IDLE: state <= RUN;
            RUN:  state <= DONE;
            default: state <= IDLE;
        endcase

    always_comb begin
        priority casez (state)
            2'b0?: m.req = 1'b1;
            default: m.req = 1'b0;
        endcase
    end

    always_latch if (clk) big = 0;

    initial begin
        foreach (arr[i]) arr[i] = i;
        do s++; while (s < 4);
        forever begin break; continue; end
        repeat (2) ;
        fork join fork join_any fork join_none
        wait_order(a, b);
        void'(f(1));
        assert (1) else $error("no");
        assume (1); cover (1); expect (1);
    end

    generate for (genvar i = 0; i < N; i++) begin : g end endgenerate
    final $display("done");
endmodule

program p; endprogram
class C extends B implements I; endclass
checker chk; endchecker
property pr; endproperty
sequence sq; endsequence
clocking cb; endclocking
covergroup cg; endgroup
