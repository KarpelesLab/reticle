# SDC constraints for `toggle`, written by Reticle.
# Times are in the library's time unit (nanoseconds for every
# Liberty library in use), capacitances in its capacitive load unit.
create_clock -name sys -period 4.000 [get_ports {clk}]
set_clock_uncertainty 0.050 [get_clocks {*}]
set_input_delay -clock sys 0.500 [get_ports {rst}]
set_false_path -from [get_pins {rst}]
