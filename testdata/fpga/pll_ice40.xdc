# Vivado XDC written by reticle for ice40-hx1k-tq144 (ice40)
set_property PACKAGE_PIN 21 [get_ports {clk}]
set_property IOSTANDARD LVCMOS33 [get_ports {clk}]
create_clock -name osc -period 83.333 [get_ports {clk}]
create_clock -name sys -period 20.833 [get_ports {sys}]
