# How carry_counter.txt was made: Vivado 2026.1, from this directory.
#   vivado -mode batch -source carry_counter.tcl
create_project -in_memory -part xc7a100tftg256-3
read_verilog carry_counter.v
read_xdc carry_counter.xdc
synth_design -top carry_counter -part xc7a100tftg256-3
opt_design
place_design
route_design
report_timing -to [get_pins {count_reg[31]/D}] -max_paths 1 -input_pins -file count31.rpt
report_timing -to [get_pins done_reg/D] -max_paths 1 -input_pins -file done.rpt
