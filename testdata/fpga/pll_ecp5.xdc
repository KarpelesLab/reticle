# Vivado XDC written by reticle for ecp5-45f-CABGA381 (ecp5)
set_property PACKAGE_PIN G2 [get_ports {clk}]
set_property IOSTANDARD LVCMOS33 [get_ports {clk}]
create_clock -name osc -period 40.000 [get_ports {clk}]
create_clock -name sys -period 8.000 [get_ports {sys}]
