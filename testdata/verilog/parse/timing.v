// Delay and event controls in every position: statement prefixes,
// intra-assignment, repeat-event, wait, all event-list spellings, named
// events, and continuous-assign delays.
module timing;
    reg clk, rst, a, b, c;
    reg [7:0] d, q;
    event done;
    parameter T = 10;

    always #5 clk = ~clk;
    always #(T/2) clk = ~clk;
    always #T clk = ~clk;
    always #1.5 a = b;
    always #(1:2:3) a = b;

    always @(posedge clk) q <= d;
    always @(posedge clk or negedge rst) q <= d;
    always @(posedge clk, negedge rst) q <= d;
    always @(a or b or c) d = a + b + c;
    always @(a, b, c) d = a + b + c;
    always @* d = a + b;
    always @(*) d = a + b;
    always @( * ) d = a + b;
    always @clk q <= d;
    always @(clk) q <= d;
    always @(edge clk) q <= d;
    always @(top.sub.clk) q <= d;
    always @(d[0]) a = d[0];
    always @(posedge clk) @(negedge clk) q <= d;
    always @done $display("done at %0t", $time);

    initial begin
        #10;
        #10 a = 1;
        #(10) b = 1;
        @(posedge clk);
        @(posedge clk) a = 0;
        @done;
        q = #1 d;
        q <= #1 d;
        q = @(posedge clk) d;
        q <= @(negedge clk) d;
        q = repeat (3) @(posedge clk) d;
        q <= repeat (2) @(posedge clk) d;
        wait (rst == 0);
        wait (a) b = 1;
        wait (a && b) begin
            c = 1;
        end
        -> done;
        #1 -> done;
        fork
            #5 a = 1;
            @(posedge clk) b = 1;
        join
        #100 $finish;
    end

    assign #1 a = b;
    assign #(1, 2) b = c;
    assign #(1:2:3, 4:5:6, 7:8:9) c = a;
    assign (strong1, weak0) #2 d = q;
    assign (weak0, weak1) a = b, b = c;
endmodule
