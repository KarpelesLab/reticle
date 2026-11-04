# Every line here is something Vivado does not take: it reports each, and
# carries on without it. Reticle's reading of each line is held to that in
# tests/fpga_xdc.rs. One command a line, because the answer is by line. The
# one clock is what the lines below it need in order to be refused for what
# they say rather than for naming no clock.
create_clock -period 10 [get_ports clk]
set_property PACKAGE_PIN ZZ9 [get_ports {be[0]}]
set_property PACKAGE_PIN N11 [get_ports nothing]
set_property PACKAGE_PIN N11 [get_ports {be[1] be[2]}]
set_property PACKAGE_PIN N11 [get_ports be]
set_property PACKAGE_PIN N11 [get_ports {hi[0]}]
set_property PACKAGE_PIN N11 [get_ports {a[1:0]}]
set_property PACKAGE_PIN N11 [get_ports CLK]
set_property PACKAGE_PIN N11 be
set_property PACKAGE_PIN N11 [get_ports {be\[0\]}]
set_property IOSTANDARD NONSENSE [get_ports clk]
set_property NONSENSE 1 [get_ports clk]
set_property BEL A6LUT [get_ports clk]
set_property PACKAGE_PIN N11 [get_ports clk] # not a comment
current_design nothing
foreach p {clk} { set_property PACKAGE_PIN N11 [get_ports $p] }
if {1} { set_property PACKAGE_PIN N11 [get_ports clk] }
proc pin {p} { set_property PACKAGE_PIN $p [get_ports clk] }
pin N11
source nothing.tcl
puts hello
return
nonsense_command foo
set_property PACKAGE_PIN [lindex {N11} 0] [get_ports clk]
set_property PACKAGE_PIN [string toupper n11] [get_ports clk]
create_clock -name onlyname
create_clock -period abc -name bad [get_ports rst]
create_clock -period -1 -name negative [get_ports rst]
create_clock -period 5 -name nowhere [get_ports nothing]
create_clock -period 5 -name nowhere [get_nets nothing]
set_false_path
set_false_path -from nothing
set_false_path -from [get_cells nothing]
set_false_path -from [get_cells u_sub]
set_false_path -from [get_pins m_reg/Q]
set_false_path -from [get_clocks nothing]
set_false_path -from [get_cells s1_reg]
set_false_path -from [get_cells *s1_reg]
set_false_path -to [get_cells u_sub]
set_false_path -from [get_ports so]
set_false_path -to [get_ports rst]
set_multicycle_path 2.5 -from [get_cells m_reg]
set_max_delay 5
set_max_delay 6 -datapath_only -to [get_cells m_reg]
set_input_delay -clock nothing 1 [get_ports rst]
set_input_delay -clock clk 1 [get_ports so]
set_output_delay -clock clk 1 [get_ports rst]
set_clock_groups -group clk -group clk
set_clock_groups -asynchronous -group nothing -group clk
create_pblock dup
create_pblock dup
add_cells_to_pblock [get_pblocks nothing] [get_cells m_reg]
resize_pblock [get_pblocks dup] -add {SLICE_X500Y0:SLICE_X501Y0}
set_property LOC NOPE [get_cells m_reg]
set_property LOC SLICE_X2Y2 [get_cells u_sub]
set_property BITSTREAM.CONFIG.NONSENSE 33 [current_design]
set_property CFGBVS NOPE [current_design]
set_property CONFIG_VOLTAGE 9 [current_design]
