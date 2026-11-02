# Vivado's constraints for carry_counter.v: the analyser's 400 MHz, on
# four of the ChipWhisperer-Husky Plus's own balls.
create_clock -period 2.5 -name clk [get_ports clk]
set_property PACKAGE_PIN D4 [get_ports clk]
set_property PACKAGE_PIN B7 [get_ports go]
set_property PACKAGE_PIN A7 [get_ports din]
set_property PACKAGE_PIN E2 [get_ports done]
set_property IOSTANDARD LVCMOS33 [get_ports *]
