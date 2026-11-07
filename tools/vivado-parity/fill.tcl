# What a placement is made of and what it costs, in Vivado's words, for one
# Reticle build of a design (`reticle fpga --vivado-placement <stem>.tcl`):
#   vivado -mode batch -source fill.tcl -tclargs <stage> <stem> <top> <part> <out>
#   placed:      Reticle's placement and Reticle's routes (<stem>.routes.tcl), as
#                achieved.tcl's `routes` stage.
#   netlist:     Reticle's netlist placed and routed by Vivado, place_design and
#                route_design, as achieved.tcl's `netlist` stage.
#   connections: Reticle's placement routed by Vivado with each lookup table's
#                pins free (achieved.tcl's `placement_free`); or, as
#                `connections_routes`, with Reticle's own routes.
# Into <out>, for `placed` and `netlist`: sites.txt and nets.txt as slices.tcl
# writes them, of the placement before routing; util.rpt; paths.txt, the 200 worst
# setup paths of each clock to itself, `path <clock> <slack> <data ns> <levels>`
# and under each `  pin <pin> <site> <bel> <primitive>` in order; ffs.txt, every
# flip-flop's data input, `ff <cell> <bel> <site> <site pin or -> <interconnect
# ps or -> <setup ns or -> <path net ns or -> <slack or ->`, the setup derived from
# the worst path as timing_dump.tcl derives it. For the connection stages,
# connections.txt: every routed connection between two slices, `c <driver site>
# <sink site> <fanout> <interconnect ps> <sink site pin>`. fill.py reads them.
lassign $argv stage stem top part out
file mkdir $out
cd $out
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
proc reticle_route {net tree} {
  set inner [route_string $tree]
  if {[llength [regexp -all -inline {\S+/\S+} $inner]] < 2} { return }
  set object [get_nets -quiet $net]
  if {$object ne ""} { catch {set_property FIXED_ROUTE "\{$inner \}" $object} }
}
proc or_dash {value} { if {$value eq ""} { return - } else { return $value } }
set_property design_mode GateLvl [current_fileset]
read_verilog $stem.v
read_xdc $stem.xdc
link_design -top $top -part $part
if {$stage eq "netlist"} { place_design } else { source $stem.tcl }

if {[string match connections* $stage]} {
  if {$stage eq "connections_routes"} {
    source $stem.routes.tcl
    catch { route_design -preserve }
  } else {
    foreach c [get_cells -quiet -hier -filter {LOCK_PINS != ""}] { reset_property LOCK_PINS $c }
    route_design
  }
  set f [open $out/connections.txt w]
  foreach n [get_nets -quiet -hier -filter {TYPE == SIGNAL && ROUTE_STATUS == ROUTED}] {
    set d [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == OUT}]
    if {[llength $d] != 1} continue
    set from [get_property SITE [get_cells -of_objects $d]]
    if {![string match SLICE_* $from]} continue
    set fanout [llength [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == IN}]]
    foreach dl [get_net_delays -quiet -of_objects $n -interconnect_only] {
      set p [get_pins -quiet [get_property TO_PIN $dl]]
      set to [get_property SITE [get_cells -of_objects $p]]
      if {![string match SLICE_* $to]} continue
      puts $f "c $from $to $fanout [get_property SLOW_MAX $dl] [or_dash [get_site_pins -quiet -of_objects $p]]"
    }
  }
  close $f
  puts "end"
  return
}

set f [open $out/sites.txt w]
foreach s [lsort [get_sites -quiet -of_objects [get_cells -quiet -hier -filter {IS_PRIMITIVE}]]] {
  puts $f "site $s [get_property SITE_TYPE $s] [get_tiles -of_objects $s]"
  foreach c [lsort [get_cells -quiet -of_objects $s]] {
    puts $f "  cell [get_property BEL $c] $c [get_property REF_NAME $c]"
  }
}
close $f
set f [open $out/nets.txt w]
foreach n [get_nets -quiet -hier -filter {TYPE == SIGNAL}] {
  set d [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == OUT}]
  set l [get_pins -quiet -leaf -of_objects $n -filter {DIRECTION == IN}]
  if {[llength $d] != 1} continue
  puts $f "net $n [llength $l] $d $l"
}
close $f
report_utilization -file $out/util.rpt
if {$stage eq "netlist"} {
  route_design
} else {
  source $stem.routes.tcl
  catch { route_design -preserve }
}
set f [open $out/paths.txt w]
foreach clock [get_clocks -quiet] {
  foreach p [get_timing_paths -quiet -from $clock -to $clock -max_paths 200 -nworst 1 -setup] {
    puts $f "path $clock [get_property SLACK $p] [get_property DATAPATH_DELAY $p] [get_property LOGIC_LEVELS $p]"
    foreach pin [get_pins -quiet -of_objects $p] {
      set c [get_cells -quiet -of_objects $pin]
      puts $f "  pin $pin [or_dash [get_property SITE $c]] [or_dash [get_property BEL $c]] [get_property REF_NAME $c]"
    }
  }
}
close $f
report_timing -max_paths 20 -input_pins -file $out/worst.rpt
set f [open $out/ffs.txt w]
foreach c [get_cells -quiet -hier -filter {IS_PRIMITIVE && REF_NAME =~ FD*}] {
  set d [get_pins -quiet $c/D]
  if {$d eq ""} continue
  set sp [or_dash [get_site_pins -quiet -of_objects $d]]
  set dl [get_net_delays -quiet -of_objects [get_nets -quiet -of_objects $d] -to $d -interconnect_only]
  set ic [expr {$dl eq "" ? "-" : [get_property SLOW_MAX [lindex $dl 0]]}]
  set p [get_timing_paths -quiet -to $d -max_paths 1 -setup]
  set setup -
  set net -
  set slack -
  if {$p ne ""} {
    catch {
      set setup [format %.3f [expr {[get_property ENDPOINT_CLOCK_EDGE $p] + [get_property ENDPOINT_CLOCK_DELAY $p] + [get_property CLOCK_PESSIMISM $p] - [get_property UNCERTAINTY $p] - [get_property REQUIRED_TIME $p]}]]
      set net [get_property DATAPATH_NET_DELAY $p]
      set slack [get_property SLACK $p]
    }
  }
  puts $f "ff $c [get_property BEL $c] [get_property SITE $c] $sp $ic $setup $net $slack"
}
close $f
puts "end"
