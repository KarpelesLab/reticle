# routes.tcl <stem> <top> <out> <how>: Reticle's placement (`reticle fpga
# --vivado-placement <stem>.tcl`) routed by Vivado with the lookup tables' pins
# free (<how> vivado), or with Reticle's routes fixed (<how> reticle), and written
# to <out> for routes.py: per routed signal net `n <net> <fanout> <driver site pin>
# <driver cell pin>` and `r <its ROUTE>`, per sink `s <net> <site pin> <node>...
# <interconnect delay, slow corner's maximum, ps> <cell pin>`, and the 100 worst
# setup paths of each clock to itself, `p <clock> <slack> <cell pins...>`.
#   vivado -mode batch -source routes.tcl -tclargs <stem> <top> <out> <how>
lassign $argv stem top out how
proc node_of {name} {
  global node_cache
  if {![info exists node_cache($name)]} {
    set node_cache($name) [get_nodes -quiet -of_objects [get_wires -quiet $name]]
  }
  return $node_cache($name)
}
proc walk {tree parent} {
  global children
  foreach item $tree {
    if {[string match "* *" $item]} {
      walk $item $parent
    } else {
      set node [node_of $item]
      if {$node eq ""} { error "no node for `$item`" }
      if {$node ne $parent} { lappend children($parent) $node; set parent $node }
    }
  }
}
proc written {node} {
  global children
  set out " $node"
  if {![info exists children($node)]} { return $out }
  set below $children($node)
  foreach child [lrange $below 0 end-1] { append out " \{[written $child] \}" }
  append out [written [lindex $below end]]
  return $out
}
proc route_string {tree} {
  global children
  array unset children
  walk $tree ""
  if {![info exists children()]} { return "" }
  return [written [lindex $children() 0]]
}
set routes_set 0
set routes_refused 0
proc reticle_route {net tree} {
  global routes_set routes_refused
  set inner [route_string $tree]
  if {[llength [regexp -all -inline {\S+/\S+} $inner]] < 2} { return }
  set object [get_nets -quiet $net]
  if {$object eq "" || [catch {set_property FIXED_ROUTE "\{$inner \}" $object}]} {
    incr routes_refused
  } else {
    incr routes_set
  }
}

set_property design_mode GateLvl [current_fileset]
read_verilog $stem.v
read_xdc $stem.xdc
link_design -top $top -part xc7a100tftg256-3
source $stem.tcl
if {$how eq "reticle"} {
  source $stem.routes.tcl
  catch { route_design -preserve }
} else {
  foreach c [get_cells -quiet -hier -filter {LOCK_PINS != ""}] { reset_property LOCK_PINS $c }
  route_design
}
set f [open $out w]
foreach n [get_nets -quiet -hier -filter {TYPE == SIGNAL && ROUTE_STATUS == ROUTED}] {
  set d [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == OUT}]
  if {[llength $d] != 1} continue
  set fo [llength [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == IN}]]
  puts $f "n $n $fo [get_site_pins -quiet -of_objects $d] $d"
  puts $f "r [get_property ROUTE $n]"
  foreach dl [get_net_delays -quiet -of_objects $n -interconnect_only] {
    set p [get_pins -quiet [get_property TO_PIN $dl]]
    set sp [get_site_pins -quiet -of_objects $p]
    set sn [get_nodes -quiet -of_objects $sp]
    puts $f "s $n $sp $sn [get_property SLOW_MAX $dl] $p"
  }
}
foreach clock [get_clocks -quiet] {
  foreach p [get_timing_paths -quiet -from $clock -to $clock -max_paths 100 -nworst 1 -setup] {
    set pins {}
    foreach pin [get_pins -quiet -of_objects $p] { lappend pins $pin }
    puts $f "p $clock [get_property SLACK $p] $pins"
  }
}
close $f
puts end
