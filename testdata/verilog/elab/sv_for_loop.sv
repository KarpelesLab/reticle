// A procedural for loop and a function with an early return.
module sv_for_loop(
    input  logic [7:0] d,
    output logic [3:0] ones,
    output logic [2:0] first_set
);
    function automatic logic [2:0] lowest_set(input logic [7:0] v);
        for (int i = 0; i < 8; i++) begin
            if (v[i]) return i[2:0];
        end
        return 3'd7;
    endfunction

    always_comb begin
        ones = '0;
        for (int i = 0; i < 8; i++)
            ones = ones + {3'd0, d[i]};
    end

    assign first_set = lowest_set(d);
endmodule
