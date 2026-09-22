// In Verilog-2005 the SystemVerilog keywords are plain identifiers.
module dialect;
    reg logic;
    wire bit, int, byte, string;
    integer do, ref, new, type, global;
    uwire u;
    wire always_ff = 1;

`begin_keywords "1800-2017"
    logic sv_now_keyword;
    bit b;
`end_keywords

    reg logic_again;
    reg logic;

`begin_keywords "1364-2001"
    wire uwire;
`end_keywords
endmodule
