# Path exceptions, with each kind of object at each end.
create_clock -period 10 [get_ports clk]
create_clock -period 7 -name c2 [get_ports clk2]
set_false_path -from [get_ports rst]
set_false_path -to [get_ports so]
set_false_path -from [get_ports {a[0] a[1]}] -to [get_cells m_reg]
set_false_path -from [get_cells {t_reg[*]}]
set_false_path -from [get_clocks clk] -to [get_clocks c2]
set_false_path -through [get_pins u_sub/q]
set_false_path -through [get_nets u_sub/q] -to [get_ports y0]
set_false_path -from [get_pins {r_reg[0]/C}] -to [get_pins {r_reg[1]/D}]
set_false_path -from [get_cells -hierarchical s1_reg]
set_false_path -from single
set_false_path -to {div_reg[0]}
set_false_path -from clk2
# Both a clock and a port: the clock.
set_false_path -from clk
set_false_path -setup -from [get_ports {a[2]}]
set_false_path -hold -to [get_ports y1]
set_multicycle_path 2 -from [get_cells {r_reg[2]}] -to [get_cells {r_reg[3]}]
set_multicycle_path 3 -setup -to [get_cells {t_reg[4]}]
set_multicycle_path 2 -hold -to [get_cells {t_reg[4]}]
set_multicycle_path -setup 4 -from [get_cells {div_reg[1]}]
set_multicycle_path 2 -setup -start -from [get_clocks clk] -to [get_clocks c2]
set_multicycle_path 1 -hold -end -from [get_clocks clk] -to [get_clocks c2]
set_max_delay 3 -from [get_cells {q_reg[0]}]
set_max_delay 3 -to [get_cells {q_reg[1]}]
set_max_delay 4 -from [get_cells {q_reg[2]}] -to [get_cells {q_reg[3]}]
set_max_delay 6 -datapath_only -from [get_cells u_sub/q_reg]
set_max_delay 5 -datapath_only -from [get_clocks c2] -to [get_clocks clk]
set_max_delay 3 -through [get_pins u_sub/s1_reg/D]
set_min_delay 1 -from [get_cells clk_div_reg]
set_clock_groups -asynchronous -group clk -group c2
set_clock_groups -asynchronous -group [get_clocks clk] -group [get_clocks c2]
set_clock_groups -name named -logically_exclusive -group {clk} -group {c2}
set_clock_groups -physically_exclusive -group clk -group c2
