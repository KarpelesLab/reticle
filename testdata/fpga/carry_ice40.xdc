# Vivado XDC written by reticle for ice40-hx1k-tq144 (ice40)
set_property PACKAGE_PIN 21 [get_ports {clk}]
set_property IOSTANDARD LVCMOS33 [get_ports {clk}]
set_property PACKAGE_PIN 99 [get_ports {carry}]
set_property IOSTANDARD LVCMOS33 [get_ports {carry}]
set_property PACKAGE_PIN 98 [get_ports {sum[0]}]
set_property IOSTANDARD LVCMOS33 [get_ports {sum[0]}]
create_clock -name sys -period 83.333 [get_ports {clk}]
