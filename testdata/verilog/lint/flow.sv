// Statements that cannot run, and conditions that cannot vary.
// lint-config: off:all warn:unreachable-statement warn:constant-condition
module flow (output logic y);
    function automatic int clamp(input int x);
        begin
            return x;
            clamp = 0;            // unreachable
        end
    endfunction

    initial begin
        if (1) y = 1'b0;          // always true
        while (0) y = 1'b1;       // never runs
        $finish;
        $display("never");        // unreachable
    end
endmodule
