# Vivado's constraints for analyser_fifo.v: the analyser's 400 MHz write and
# the USB interface's 96 MHz read, on the Husky Plus's part.
create_clock -period 2.5 -name wr_clk [get_ports wr_clk]
create_clock -period 10.4 -name rd_clk [get_ports rd_clk]
set_clock_groups -asynchronous -group wr_clk -group rd_clk
set_property PACKAGE_PIN D4 [get_ports wr_clk]
set_property PACKAGE_PIN E3 [get_ports rd_clk]
set_property PACKAGE_PIN B7 [get_ports din]
set_property PACKAGE_PIN A7 [get_ports wr_en]
set_property PACKAGE_PIN E2 [get_ports rd_en]
set_property PACKAGE_PIN C1 [get_ports dout]
set_property PACKAGE_PIN B6 [get_ports full]
set_property PACKAGE_PIN A5 [get_ports empty]
set_property IOSTANDARD LVCMOS33 [get_ports *]
