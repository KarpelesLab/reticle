# The period each clock of one design achieves under Vivado's timer, written to
# <out>/<top>.result as `clock <name> period <p> slack <s> achieved <p - s> ...`.
#   vivado -mode batch -source achieved.tcl -tclargs <stage> <stem> <top> <part> <out>
#   own:            <stem>.v and <stem>.xdc through Vivado's own flow: synth_design,
#                   opt_design, place_design, phys_opt_design, route_design.
#   netlist:        Reticle's netlist (`reticle fpga --vivado-placement <stem>.tcl`)
#                   placed and routed by Vivado: place_design and route_design only.
#   vnetlist:       Vivado's own netlist of <stem>.v placed and routed the same way:
#                   synth_design, then place_design and route_design only, so that
#                   beside `netlist` only the netlist differs.
#   placement:      Reticle's cells where Reticle put them, routed by Vivado with each
#                   lookup table's pins locked where Reticle's router put them.
#   placement_free: the same with the pins free, Vivado's router choosing them.
#   routes:         Reticle's routes, as each net's FIXED_ROUTE (run_placed.tcl's).
# A clock's achieved period is its period less the worst setup slack of the paths
# from it to itself: paths between two clocks are left out, as Reticle times none.
lassign $argv stage stem top part out
file mkdir $out
cd $out
set result [open $top.result w]
proc say {args} { global result; puts $result [join $args " "]; flush $result }
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
say stage $stage part $part vivado [version -short]
set t0 [clock seconds]
if {$stage eq "own"} {
  read_verilog $stem.v
  read_xdc $stem.xdc
  synth_design -top $top -part $part
  opt_design
  place_design
  phys_opt_design
  route_design
} elseif {$stage eq "vnetlist"} {
  read_verilog $stem.v
  read_xdc $stem.xdc
  synth_design -top $top -part $part
  place_design
  route_design
} else {
  set_property design_mode GateLvl [current_fileset]
  read_verilog $stem.v
  read_xdc $stem.xdc
  link_design -top $top -part $part
  if {$stage eq "netlist"} {
    place_design
    route_design
  } elseif {$stage eq "placement" || $stage eq "placement_free"} {
    source $stem.tcl
    if {$stage eq "placement_free"} {
      foreach c [get_cells -quiet -hier -filter {LOCK_PINS != ""}] { reset_property LOCK_PINS $c }
    }
    route_design
  } else {
    source $stem.tcl
    source $stem.routes.tcl
    say routes_set $routes_set refused $routes_refused
    # A ground net Vivado cannot finish around fixed routes is no path.
    if {[catch { route_design -preserve }]} { say route_rest_refused }
  }
}
say seconds [expr {[clock seconds] - $t0}]
set unrouted [get_nets -quiet -hier -filter {ROUTE_STATUS != INTRASITE && ROUTE_STATUS != ROUTED && ROUTE_STATUS != NOLOADS && ROUTE_STATUS != NODRIVER}]
say nets_not_routed [llength $unrouted] [lrange $unrouted 0 4]
foreach clock [get_clocks -quiet] {
  set path [get_timing_paths -quiet -from $clock -to $clock -max_paths 1 -setup]
  if {[llength $path]} {
    set period [get_property PERIOD $clock]
    set slack [get_property SLACK $path]
    say clock $clock period $period slack $slack achieved [format %.3f [expr {$period - $slack}]] delay [get_property DATAPATH_DELAY $path] skew [get_property SKEW $path] levels [get_property LOGIC_LEVELS $path] from [get_property STARTPOINT_PIN $path] to [get_property ENDPOINT_PIN $path]
  }
}
report_timing -max_paths 3 -input_pins -file [file join [pwd] worst.rpt]
say end
