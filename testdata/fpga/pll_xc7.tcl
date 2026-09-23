# Vivado batch script written by reticle for xc7a35t-cpg236 (xc7)
# Run it with: vivado -mode batch -source pllshift.tcl
# The netlist is already mapped to 7-series primitives, so this
# elaborates and checks the instantiations rather than inferring
# anything. Reticle has not run it: what the test suite proves is
# that the cells and pins are ones the device database declares.
create_project -in_memory -part xc7a35tcpg236-1
read_verilog pllshift.v
read_xdc pllshift.xdc
synth_design -top pllshift -part xc7a35tcpg236-1
opt_design
place_design
route_design
report_utilization -file pllshift_utilization.rpt
report_timing_summary -file pllshift_timing.rpt
write_bitstream -force pllshift.bit
