create_clock -period 5 -name clk [get_ports clk]
set_property PACKAGE_PIN N11 [get_ports clk]
set_property PACKAGE_PIN L4 [get_ports si]
set_property PACKAGE_PIN L5 [get_ports so]
set_property IOSTANDARD LVCMOS33 [get_ports *]
create_clock -period 7 -name clk2 [get_ports clk2]
set_property PACKAGE_PIN N14 [get_ports clk2]
set_clock_groups -asynchronous -group clk -group clk2
