# What Vivado's timer does with the clock at both ends of a setup path, on a
# routed checkpoint (achieved.sh's `routes` stage with KEEP_DCP, or
# regrade.sh's): every global clock pin's interconnect delay at the slow
# corner's maximum and minimum, and each clock's 400 worst setup paths, one an
# endpoint, with the timer's own clock terms.
#   vivado -mode batch -source clock_pessimism.tcl -tclargs <checkpoint> <out>
# Writes <out>, one fact a line:
#   part <part>
#   pin <net> <clock pin> <site> <clock region> <slow max ps> <slow min ps>
#       <fast max ps> <fast min ps>
#   path <clock> <start pin> <site> <region> <end pin> <site> <region>
#       slack <ns> skew <ns> scd <ns> dcd <ns> cpr <ns> unc <ns> delay <ns>
#       req <ns> corner <corner>
# where scd and dcd are the source's and the destination's clock delay, cpr the
# clock pessimism removal and unc the uncertainty. clock_pessimism.py reads it.
lassign $argv dcp out
open_checkpoint $dcp
set f [open $out w]
puts $f "part [get_property PART [current_design]]"
foreach net [get_nets -hier -filter {TYPE == GLOBAL_CLOCK}] {
  foreach dl [get_net_delays -of_objects $net -interconnect_only] {
    set p [get_pins [get_property TO_PIN $dl]]
    set s [get_property SITE [get_cells -of_objects $p]]
    puts $f "pin $net $p $s [get_property CLOCK_REGION [get_sites $s]] [get_property SLOW_MAX $dl] [get_property SLOW_MIN $dl] [get_property FAST_MAX $dl] [get_property FAST_MIN $dl]"
  }
}
foreach clock [get_clocks -quiet] {
  foreach p [get_timing_paths -quiet -from $clock -to $clock -max_paths 400 -nworst 1 -setup] {
    set sp [get_property STARTPOINT_PIN $p]
    set ep [get_property ENDPOINT_PIN $p]
    set ss [get_property SITE [get_cells -of_objects $sp]]
    set es [get_property SITE [get_cells -of_objects $ep]]
    puts $f "path $clock $sp $ss [get_property CLOCK_REGION [get_sites $ss]] $ep $es [get_property CLOCK_REGION [get_sites $es]] slack [get_property SLACK $p] skew [get_property SKEW $p] scd [get_property STARTPOINT_CLOCK_DELAY $p] dcd [get_property ENDPOINT_CLOCK_DELAY $p] cpr [get_property CLOCK_PESSIMISM $p] unc [get_property UNCERTAINTY $p] delay [get_property DATAPATH_DELAY $p] req [get_property REQUIREMENT $p] corner [get_property CORNER $p]"
  }
}
close $f
puts end
