# Vivado's timer, element by element, on a routed checkpoint: what
# timing_table.py fits Reticle's delay model to.
#   vivado -mode batch -source timing_dump.tcl -tclargs <checkpoint> <out>
# Writes <out>, one fact a line, all delays the slow corner's maximum:
#   node <node> <cost code name>
#   pip <net> <node above> <node below> <pip>
#   sink <net> <pin> <its node> <ps>          interconnect delay to that pin
#   source <net> <pin> <its node>
#   arc <cell> <ref> <from> <to> <type> <rise ns> <fall ns>
#   lock <cell> <logical pin>:<physical pin> ...
#   path <start clock> <end clock> <from> <to> <data ns> <logic ns> <route ns>
#        <slack> <skew> <setup ns> <levels>     every endpoint's worst path
# Only answers come out: delays of a design written here, on Reticle's routes.
lassign $argv checkpoint out
open_checkpoint $checkpoint
set f [open $out w]
set nodes [dict create]
foreach net [get_nets -quiet -hier -filter {TYPE == SIGNAL && ROUTE_STATUS == ROUTED}] {
  foreach pip [get_pips -quiet -of_objects $net] {
    set up [get_nodes -quiet -uphill -of_objects $pip]
    set down [get_nodes -quiet -downhill -of_objects $pip]
    puts $f "pip $net $up $down $pip"
    foreach node [list $up $down] {
      if {$node ne "" && ![dict exists $nodes $node]} {
        dict set nodes $node [get_property COST_CODE_NAME $node]
      }
    }
  }
  foreach pin [get_pins -quiet -of_objects $net -filter {DIRECTION == OUT}] {
    set node [get_nodes -quiet -of_objects [get_site_pins -quiet -of_objects $pin]]
    if {$node ne ""} { puts $f "source $net $pin [lindex $node 0]" }
  }
  foreach delay [get_net_delays -quiet -of_objects $net -interconnect_only] {
    set pin [get_pins -quiet [get_property TO_PIN $delay]]
    set node [get_nodes -quiet -of_objects [get_site_pins -quiet -of_objects $pin]]
    if {$node eq ""} { set node - }
    puts $f "sink $net $pin [lindex $node 0] [get_property SLOW_MAX $delay]"
  }
}
dict for {node code} $nodes { puts $f "node $node $code" }
foreach cell [get_cells -quiet -hier -filter {IS_PRIMITIVE}] {
  set ref [get_property REF_NAME $cell]
  set lock [get_property -quiet LOCK_PINS $cell]
  if {$lock ne ""} { puts $f "lock $cell $lock" }
  foreach arc [get_timing_arcs -quiet -of_objects $cell] {
    set type [get_property TYPE $arc]
    if {$type eq "hold" || [get_property IS_DISABLED $arc]} { continue }
    set from [get_property REF_PIN_NAME [get_property FROM_PIN $arc]]
    set to [get_property REF_PIN_NAME [get_property TO_PIN $arc]]
    puts $f "arc $cell $ref $from $to [string map {" " _} $type] [get_property DELAY_SLOW_MAX_RISE $arc] [get_property DELAY_SLOW_MAX_FALL $arc]"
  }
}
foreach clock [get_clocks -quiet] {
  foreach path [get_timing_paths -quiet -group $clock -max_paths 100000 -nworst 1] {
    set setup [expr {[get_property ENDPOINT_CLOCK_EDGE $path] + [get_property ENDPOINT_CLOCK_DELAY $path] + [get_property CLOCK_PESSIMISM $path] - [get_property UNCERTAINTY $path] - [get_property REQUIRED_TIME $path]}]
    puts $f "path [get_property STARTPOINT_CLOCK $path] [get_property ENDPOINT_CLOCK $path] [get_property STARTPOINT_PIN $path] [get_property ENDPOINT_PIN $path] [get_property DATAPATH_DELAY $path] [get_property DATAPATH_LOGIC_DELAY $path] [get_property DATAPATH_NET_DELAY $path] [get_property SLACK $path] [get_property SKEW $path] [format %.3f $setup] [get_property LOGIC_LEVELS $path]"
  }
}
close $f
