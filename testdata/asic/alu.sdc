# SDC constraints for `alu`, written by Reticle.
# Times are in the library's time unit (nanoseconds for every
# Liberty library in use), capacitances in its capacitive load unit.
create_clock -name virt -period 4.000 [get_ports {a}]
set_clock_uncertainty 0.050 [get_clocks {*}]
set_input_delay -clock virt 0.400 [get_ports {a}]
set_output_delay -clock virt 0.400 [get_ports {y}]
set_load 0.01 [get_ports {y}]
