# Input and output delays, in each combination of their options.
create_clock -period 10 [get_ports clk]
create_clock -period 7 -name c2 [get_ports clk2]
create_clock -period 20 -name virt
set_input_delay -clock clk 2 [get_ports rst]
set_input_delay -clock clk -max 2 [get_ports single]
set_input_delay -clock clk -min 1 [get_ports single]
set_input_delay -clock virt 1.5 [get_ports {a[0]}]
set_input_delay -clock clk -clock_fall 1.5 [get_ports {a[1]}]
set_input_delay -clock clk 1 [get_ports {a[2]}]
set_input_delay -clock c2 1 -add_delay [get_ports {a[2]}]
set_input_delay -clock [get_clocks clk] 1 [get_ports {a[3]}]
set_input_delay -clock clk 1 {be[0]}
set_input_delay -clock clk 2.5 [get_ports hi[*]]
set_output_delay -clock clk 2 [get_ports so]
set_output_delay -clock clk -max 2 [get_ports y0]
set_output_delay -clock clk -min -1 [get_ports y0]
