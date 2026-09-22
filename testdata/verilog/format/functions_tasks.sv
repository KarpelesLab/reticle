// Functions and tasks: ANSI and non-ANSI ports, return types (implicit,
// ranged, void, typed), lifetimes, defaults, ref and inout arguments,
// declarations in bodies, and calls.
module subs;
    function [7:0] add8;
        input [7:0] a, b;
        begin
            add8 = a + b;
        end
    endfunction

    function automatic integer fact(input integer n);
        if (n <= 1) fact = 1;
        else fact = n * fact(n - 1);
    endfunction

    function signed [15:0] neg(input signed [15:0] x);
        neg = -x;
    endfunction

    function void log(string msg, int level = 0);
        $display("[%0d] %s", level, msg);
    endfunction

    function int sum(int arr[], ref int count);
        sum = 0;
        count = 0;
        foreach (arr[k]) begin
            sum += arr[k];
            count++;
        end
    endfunction

    function static bit parity(input bit [7:0] v);
        parity = ^v;
    endfunction

    function logic [3:0] nibble(logic [7:0] v, bit hi);
        return hi ? v[7:4] : v[3:0];
    endfunction

    function real half(real x);
        return x / 2.0;
    endfunction

    function noargs;
        noargs = 1'b1;
    endfunction

    function pkg::word_t typed_ret(input a);
        typed_ret = a;
    endfunction

    task automatic pulse(inout logic sig, input int width = 1, output int done);
        int i;
        logic saved = sig;
        for (i = 0; i < width; i++) begin
            sig = ~sig;
            #1;
        end
        sig = saved;
        done = 1;
    endtask

    task old_style;
        input [3:0] a;
        output [3:0] b;
        inout c;
        reg tmp;
        begin
            tmp = c;
            b = a;
        end
    endtask : old_style

    task no_ports;
        $display("no ports");
    endtask

    task with_ref(ref int r, const ref int cr);
        r = cr;
    endtask

    int d;
    logic s;
    initial begin
        int arr[] = new[3];
        int cnt;
        d = fact(5);
        log("start");
        log(.msg("named"), .level(2));
        pulse(s, 3, d);
        pulse(.sig(s), .done(d));
        void'(sum(arr, cnt));
        old_style(4'd1, d[3:0], s);
        no_ports;
        no_ports();
        d = nibble(8'hab, 1);
    end
endmodule
