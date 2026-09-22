# SDC constraints for `counter`, written by Reticle.
# Times are in the library's time unit (nanoseconds for every
# Liberty library in use), capacitances in its capacitive load unit.
create_clock -name sys -period 4.000 [get_ports {clk}]
set_clock_uncertainty 0.050 [get_clocks {*}]
set_input_delay -clock sys 0.500 [get_ports {rst}]
set_input_delay -clock sys 0.500 [get_ports {en}]
set_output_delay -clock sys 0.800 [get_ports {q}]
set_load 0.01 [get_ports {q}]
set_driving_cell -lib_cell INV_X1 -pin Y [get_ports {en}]
set_false_path -from [get_pins {rst}]
set_multicycle_path 2 -setup -from [get_pins {u_reg}] -to [get_pins {carry}]
