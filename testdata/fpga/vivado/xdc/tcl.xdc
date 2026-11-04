# How much of Tcl an XDC has: what Vivado accepts in a constraints file read
# by read_xdc, which is less than a Tcl shell has.
   # An indented comment.
current_design
set_property PACKAGE_PIN N11 [get_ports clk];# a comment after a semicolon
set_property PACKAGE_PIN \
    N14 \
    [get_ports clk2]
set_property PACKAGE_PIN L4 [get_ports rst]; set_property PACKAGE_PIN L5 [get_ports single]
# A comment continued onto the next line \
set_property PACKAGE_PIN K5 [get_ports so]
set pin K3
set_property PACKAGE_PIN $pin [get_ports {a[0]}]
set port {a[1]}
set_property PACKAGE_PIN J4 [get_ports $port]
set_property PACKAGE_PIN J5 [get_ports ${port}]
set_property PACKAGE_PIN H[expr {3+1}] [get_ports {a[2]}]
set_property PACKAGE_PIN [join {H 5} ""] [get_ports {a[3]}]
set_property PACKAGE_PIN [lindex [list T2] 0] [get_ports {hi[4]}]
set_property PACKAGE_PIN [list T3] [get_ports {hi[5]}]
set_property PACKAGE_PIN [expr {1 ? "R5" : "R6"}] [get_ports {hi[6]}]
set period [expr {2.5*2}]
create_clock -period $period -name sys [get_ports clk]
set_property IOSTANDARD [get_property IOSTANDARD [get_ports rst]] [get_ports {hi[7]}]
set_property -quiet IOSTANDARD LVCMOS33 [get_ports {be[0]}]
set_property IOSTANDARD LVCMOS33 [get_ports -quiet {be[1]}] -verbose
