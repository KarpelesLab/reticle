# Vivado XDC written by reticle for ecp5-45f-CABGA381 (ecp5)
set_property PACKAGE_PIN G2 [get_ports {clk}]
set_property IOSTANDARD LVCMOS33 [get_ports {clk}]
set_property DRIVE 8 [get_ports {clk}]
set_property PACKAGE_PIN B2 [get_ports {we}]
set_property IOSTANDARD LVCMOS33 [get_ports {we}]
set_property PACKAGE_PIN C1 [get_ports {rdata[0]}]
set_property IOSTANDARD LVCMOS33 [get_ports {rdata[0]}]
create_clock -name sys -period 40.000 [get_ports {clk}]
