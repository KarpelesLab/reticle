// A procedural assignment to a net.
module error_proc_assign_net(input wire clk, input wire d, output wire q);
    always @(posedge clk)
        q <= d;
endmodule
