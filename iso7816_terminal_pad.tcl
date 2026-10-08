# Vivado batch script written by reticle for xc7a35t-cpg236 (xc7)
# Run it with: vivado -mode batch -source iso7816_terminal_pad.tcl
# The netlist is already mapped to 7-series primitives, so this
# elaborates and checks the instantiations rather than inferring
# anything. Reticle has not run it: what the test suite proves is
# that the cells and pins are ones the device database declares.
create_project -in_memory -part {xc7a35tcpg236-1}
read_verilog {iso7816_terminal_pad.v}
read_xdc {iso7816_terminal_pad.xdc}
synth_design -top {iso7816_terminal_pad} -part {xc7a35tcpg236-1}
opt_design
place_design
route_design
report_utilization -file {iso7816_terminal_pad_utilization.rpt}
report_timing_summary -file {iso7816_terminal_pad_timing.rpt}
write_bitstream -force {iso7816_terminal_pad.bit}
