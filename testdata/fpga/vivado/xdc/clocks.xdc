# Each way of saying what a clock is.
create_clock -period 10 [get_ports clk]
create_clock -period 7.5 -name c2 -waveform {0 3} [get_ports clk2]
create_clock -period 20 -name virt
create_clock -period 8 -name c2add -add [get_ports clk2]
# A register as a clock, by each of its options.
create_generated_clock -name g_div -source [get_ports clk] -divide_by 2 [get_pins clk_div_reg/Q]
create_generated_clock -name g_scale -source [get_pins {div_reg[0]/C}] -divide_by 4 -multiply_by 3 [get_pins {div_reg[1]/Q}]
create_generated_clock -name g_edges -source [get_ports clk] -edges {1 3 5} [get_pins m_reg/Q]
create_generated_clock -source [get_ports clk] -divide_by 2 -invert [get_pins {q_reg[0]/Q}]
# A clock manager's output, which Vivado would derive anyway.
create_generated_clock -name g_mmcm -source [get_pins u_mmcm/CLKIN1] -multiply_by 2 [get_pins u_mmcm/CLKOUT0]
# Two statements of one name: the later one stays.
create_clock -period 30 -name twice [get_ports rst]
create_clock -period 31 -name twice [get_ports rst]
# A later clock on a source takes it from the older one, whose other sources
# stay; an older one left with none is gone.
create_clock -period 40 -name both [get_ports {a[0] a[1]}]
create_clock -period 41 -name part [get_ports {a[0]}]
create_clock -period 42 -name gone [get_ports single]
create_clock -period 43 -name taker [get_ports single]
