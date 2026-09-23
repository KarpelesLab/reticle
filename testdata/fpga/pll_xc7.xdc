# Vivado XDC written by reticle for xc7a35t-cpg236 (xc7)
set_property PACKAGE_PIN W5 [get_ports {clk}]
set_property IOSTANDARD LVCMOS33 [get_ports {clk}]
set_property PACKAGE_PIN V17 [get_ports {d}]
set_property IOSTANDARD LVCMOS33 [get_ports {d}]
set_property PACKAGE_PIN U16 [get_ports {q}]
set_property IOSTANDARD LVCMOS33 [get_ports {q}]
create_clock -name osc -period 10.000 [get_ports {clk}]
create_clock -name sys -period 20.000 [get_ports {sys}]
