# How analyser_fifo.txt was made: Vivado 2026.1, from this directory.
#   vivado -mode batch -source analyser_fifo.tcl
create_project -in_memory -part xc7a100tftg256-3
read_verilog {analyser_fifo.v ../../../examples/husky/fifo_generator.v}
read_xdc analyser_fifo.xdc
synth_design -top analyser_fifo -part xc7a100tftg256-3
opt_design
place_design
route_design
report_timing -group wr_clk -max_paths 1 -input_pins -file wr.rpt
report_timing -group rd_clk -max_paths 1 -input_pins -file rd.rpt
report_timing -to [get_pins -hier -filter {NAME =~ *wused_q_reg[14]/D}] -max_paths 1 -input_pins -file wused.rpt
report_timing -to [get_pins -hier -filter {NAME =~ *u_fifo/stored_reg/D}] -max_paths 1 -input_pins -file stored.rpt
report_timing -to [get_pins dout_reg/D] -max_paths 1 -input_pins -file dout.rpt
