# Vivado batch script written by reticle for xc7a35t-cpg236 (xc7)
# Run it with: vivado -mode batch -source ram16.tcl
# The netlist is already mapped to 7-series primitives, so this
# elaborates and checks the instantiations rather than inferring
# anything. Reticle has not run it: what the test suite proves is
# that the cells and pins are ones the device database declares.
create_project -in_memory -part xc7a35tcpg236-1
read_verilog ram16.v
read_xdc ram16.xdc
synth_design -top ram16 -part xc7a35tcpg236-1
opt_design
place_design
route_design
report_utilization -file ram16_utilization.rpt
report_timing_summary -file ram16_timing.rpt
write_bitstream -force ram16.bit
