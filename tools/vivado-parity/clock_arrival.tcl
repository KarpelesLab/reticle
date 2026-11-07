# When a global clock reaches each slice of the xc7a100t: a chain of flip-flops,
# one at `AFF` of every even slice column on three rows of each clock region row
# (the edges and the middle), one global buffer held to <buffer site>, routed,
# and the clock net's interconnect delay to every clock pin.
#   vivado -mode batch -source clock_arrival.tcl -tclargs <buffer site> <out stem>
# Writes <out stem>.v, the design, and <out stem>.txt: `buffer <site>`, then a
# line `clk <site> <tile> <slow max ps> <fast min ps>` per flip-flop.
# clock_arrival.py reads it.
lassign $argv buffer out
set n 700
set f [open $out.v w]
puts $f "module skew(input wire clk, input wire d, output wire q);"
puts $f "  wire c; BUFG b(.I(clk), .O(c));"
puts $f "  (* keep = \"true\" *) reg \[[expr {$n-1}]:0\] r;"
puts $f "  always @(posedge c) r <= {r\[[expr {$n-2}]:0\], d};"
puts $f "  assign q = r\[[expr {$n-1}]\];"
puts $f "endmodule"
close $f
read_verilog $out.v
synth_design -top skew -part xc7a100tftg256-3 -flatten_hierarchy full
set_property PACKAGE_PIN N11 [get_ports clk]
set_property PACKAGE_PIN L4 [get_ports d]
set_property PACKAGE_PIN L5 [get_ports q]
set_property IOSTANDARD LVCMOS33 [get_ports *]
create_clock -period 5 [get_ports clk]
set_property LOC $buffer [get_cells b]
# The pad's way to a buffer of the other half is not what is measured.
set_property CLOCK_DEDICATED_ROUTE FALSE [get_nets -of_objects [get_pins b/I]]
set rows {1 24 48 51 74 98 101 124 148 151 174 198}
set sites {}
foreach s [get_sites -filter {SITE_TYPE == SLICEL || SITE_TYPE == SLICEM}] {
  regexp {SLICE_X(\d+)Y(\d+)} $s - x y
  if {$x % 2 == 0 && [lsearch -exact $rows $y] >= 0} { lappend sites $s }
}
set cells [lsort -dictionary [get_cells -hier -filter {REF_NAME == FDRE}]]
set i 0
foreach s $sites {
  if {$i >= [llength $cells]} break
  place_cell [lindex $cells $i] $s/AFF
  incr i
}
place_design
route_design
set f [open $out.txt w]
puts $f "buffer $buffer"
foreach net [get_nets -hier -filter {TYPE == GLOBAL_CLOCK}] {
  foreach dl [get_net_delays -of_objects $net -interconnect_only] {
    set c [get_cells -of_objects [get_pins [get_property TO_PIN $dl]]]
    set s [get_property SITE $c]
    puts $f "clk $s [get_tiles -of_objects [get_sites $s]] [get_property SLOW_MAX $dl] [get_property FAST_MIN $dl]"
  }
}
close $f
puts end
