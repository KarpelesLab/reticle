# When a global clock reaches each slice of a part: a chain of flip-flops,
# one at `AFF` of every even slice column on three rows of each clock region row
# (the edges and the middle), one global buffer held to <buffer site>, routed,
# and the clock net's interconnect delay to every clock pin.
#   vivado -mode batch -source clock_arrival.tcl -tclargs <buffer site> <out stem> \
#       [<part> <clk ball> <d ball> <q ball>]
# The part is xc7a100tftg256-3 and the balls N11, L4 and L5 unless given; the
# XC7A35T's FTG256 takes the same balls, its CPG236 N17, R2 and U1.
# Writes <out stem>.v, the design, and <out stem>.txt: `buffer <site>`, then a
# line `clk <site> <tile> <slow max ps> <fast min ps>` per flip-flop.
# clock_arrival.py reads it.
lassign $argv buffer out part clk_ball d_ball q_ball
if {$part eq ""} { set part xc7a100tftg256-3 }
if {$clk_ball eq ""} { lassign {N11 L4 L5} clk_ball d_ball q_ball }
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
synth_design -top skew -part $part -flatten_hierarchy full
set_property PACKAGE_PIN $clk_ball [get_ports clk]
set_property PACKAGE_PIN $d_ball [get_ports d]
set_property PACKAGE_PIN $q_ball [get_ports q]
set_property IOSTANDARD LVCMOS33 [get_ports *]
create_clock -period 5 [get_ports clk]
set_property LOC $buffer [get_cells b]
# The pad's way to a buffer of the other half is not what is measured.
set_property CLOCK_DEDICATED_ROUTE FALSE [get_nets -of_objects [get_pins b/I]]
# Three rows of each clock region row, fifty slices tall: its edges and middle.
set slices [get_sites -filter {SITE_TYPE == SLICEL || SITE_TYPE == SLICEM}]
set top 0
foreach s $slices { regexp {SLICE_X\d+Y(\d+)} $s - y; if {$y > $top} { set top $y } }
set rows {}
for {set base 0} {$base <= $top} {incr base 50} {
  lappend rows [expr {$base + 1}] [expr {$base + 24}] [expr {$base + 48}]
}
set sites {}
foreach s $slices {
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
puts $f "part $part"
foreach net [get_nets -hier -filter {TYPE == GLOBAL_CLOCK}] {
  foreach dl [get_net_delays -of_objects $net -interconnect_only] {
    set c [get_cells -of_objects [get_pins [get_property TO_PIN $dl]]]
    set s [get_property SITE $c]
    puts $f "clk $s [get_tiles -of_objects [get_sites $s]] [get_property SLOW_MAX $dl] [get_property FAST_MIN $dl]"
  }
}
close $f
puts end
