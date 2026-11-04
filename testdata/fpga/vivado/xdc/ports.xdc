# What a port can be told, and how a query names the port.
# Bare words, braces and quotes all name a bit of a bus.
set_property PACKAGE_PIN N11 [get_ports clk]
set_property PACKAGE_PIN N14 [get_ports {clk2}]
set_property PACKAGE_PIN L4 [get_ports a[0]]
set_property PACKAGE_PIN L5 [get_ports {a[1]}]
set_property PACKAGE_PIN K5 [get_ports "a[2]"]
set_property PACKAGE_PIN K3 [get_ports a\[3\]]
# A bus not numbered from zero is named by the index it was declared with.
set_property PACKAGE_PIN J4 [get_ports {hi[4]}]
set_property PACKAGE_PIN J5 [get_ports {hi[7]}]
set_property PACKAGE_PIN H4 [get_ports {be[0]}]
set_property PACKAGE_PIN H5 [get_ports {be[3]}]
# Case is not significant in a property's name or in a pin, and a lower-case
# standard is the upper-case one.
set_property package_pin t2 [get_ports rst]
set_property IOSTANDARD lvcmos25 [get_ports rst]
# The later statement wins.
set_property PACKAGE_PIN G1 [get_ports single]
set_property PACKAGE_PIN G2 [get_ports single]
# LOC with a package pin's name is the same thing.
set_property LOC T3 [get_ports io1]
# -dict, in each of its three spellings.
set_property -dict {PACKAGE_PIN R5 IOSTANDARD LVCMOS18 DRIVE 8 SLEW FAST} [get_ports so]
set_property -dict "PACKAGE_PIN R6 IOSTANDARD LVCMOS18" [get_ports y0]
set_property -dict [list PACKAGE_PIN R7 IOSTANDARD LVCMOS18 DRIVE 4] [get_ports y1]
# Globs: a bus's bare name is every bit of it, and * and ? work.
set_property IOSTANDARD LVCMOS33 [get_ports a]
set_property IOSTANDARD LVCMOS33 [get_ports hi[*]]
set_property IOSTANDARD LVCMOS33 [get_ports ?e[*]]
set_property IOSTANDARD LVCMOS33 [get_ports {clk clk2}]
set_property IOSTANDARD LVCMOS33 [get_ports {single io1}]
set_property IOSTANDARD LVCMOS33 [get_ports [list q[0] q[1]]]
set_property IOSTANDARD LVCMOS33 [list [get_ports q[2]] [get_ports q[3]]]
set_property IOSTANDARD LVCMOS33 [get_ports y2] [get_ports y3]
# Queries by property, and the two collections.
set_property PULLUP true [get_ports -filter {NAME =~ a[1]}]
set_property SLEW SLOW [get_ports -filter {DIRECTION == OUT && NAME == y1}]
set_property PULLDOWN yes [filter [get_ports *] {NAME == single}]
set_property DRIVE 16 [all_outputs]
# What PULLTYPE and the two flags make of each other.
set_property PULLUP 1 [get_ports {a[0]}]
set_property PULLDOWN 1 [get_ports {a[0]}]
set_property PULLUP 1 [get_ports {a[2]}]
set_property PULLUP false [get_ports {a[2]}]
set_property PULLDOWN 1 [get_ports {a[3]}]
set_property PULLUP false [get_ports {a[3]}]
set_property PULLTYPE PULLUP [get_ports {hi[4]}]
# And a glob on a bus's own name.
set_property IOSTANDARD LVCMOS25 [get_ports h?]
set_property PULLTYPE PULLDOWN [get_ports {hi[5]}]
set_property PULLDOWN 1 [get_ports {hi[6]}]
set_property PULLUP 1 [get_ports {hi[6]}]
# An input has no drive or slew: Vivado warns, and sets neither.
set_property DRIVE 8 [get_ports clk]
set_property SLEW FAST [get_ports clk2]
