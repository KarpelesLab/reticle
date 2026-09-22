`timescale 1ns / 1ps
`default_nettype none
`resetall
`celldefine
module dir;
    wire w;
endmodule
`endcelldefine
`unconnected_drive pull1
`nounconnected_drive
`default_decay_time 10
`default_trireg_strength 50
`delay_mode_path
`pragma protect begin
`protect
`endprotect
`line 100 "elsewhere.v" 0
`timescale 10 ps/1 ps
localparam string F = `__FILE__;
localparam L = `__LINE__;
`default_nettype wire
