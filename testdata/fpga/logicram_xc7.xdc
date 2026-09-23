# Vivado XDC written by reticle for xc7a35t-cpg236 (xc7)
set_property PACKAGE_PIN W5 [get_ports {clk}]
set_property IOSTANDARD LVCMOS33 [get_ports {clk}]
set_property PACKAGE_PIN V17 [get_ports {we}]
set_property IOSTANDARD LVCMOS33 [get_ports {we}]
create_clock -name sys -period 10.000 [get_ports {clk}]
