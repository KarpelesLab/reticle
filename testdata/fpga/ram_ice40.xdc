# Vivado XDC written by reticle for ice40-hx1k-tq144 (ice40)
set_property PACKAGE_PIN 21 [get_ports {clk}]
set_property IOSTANDARD LVCMOS33 [get_ports {clk}]
set_property PACKAGE_PIN 8 [get_ports {we}]
set_property IOSTANDARD LVCMOS33 [get_ports {we}]
set_property PACKAGE_PIN 95 [get_ports {rdata[0]}]
set_property IOSTANDARD LVCMOS33 [get_ports {rdata[0]}]
create_pblock core
resize_pblock [get_pblocks core] -add {SLICE_X1Y1:SLICE_X6Y6}
add_cells_to_pblock [get_pblocks core] [get_cells {rd}]
set_property KEEP_HIERARCHY TRUE [get_cells {wr}]
create_clock -name sys -period 83.333 [get_ports {clk}]
set_multicycle_path 2 -setup -from [get_cells {waddr}] -to [get_ports {rdata}]
