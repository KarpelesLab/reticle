# Vivado's timer on Reticle's implementation of one design, with what it found
# written to <top>.result in the form run_vivado.tcl uses.
#   vivado -mode batch -source run_placed.tcl -tclargs <stem> <top> <part> <out> <stage>
# <stem> names what `reticle fpga --vivado-placement <stem>.tcl` wrote.
#   netlist:   Reticle's netlist, placed and routed by Vivado, which is what says
#              how much of a gap is the placement and how much the netlist.
#   placement: Reticle's cells where Reticle put them, routed by Vivado.
#   routes:    and Reticle's routes, as each net's FIXED_ROUTE.
# Neither runs synthesis or opt_design, so nothing here changes what is timed.
lassign $argv stem top part out stage
file mkdir $out
cd $out
set result [open $top.result w]
proc say {args} { global result; puts $result [join $args " "]; flush $result }
proc step {name script} {
  set t0 [clock milliseconds]
  if {[catch {uplevel 1 $script} message]} {
    say failed $name
    say message [string map {"\n" " "} $message]
    say end
    exit 0
  }
  say seconds $name [format %.3f [expr {([clock milliseconds] - $t0) / 1000.0}]]
}

# Reticle's `<tile>/<wire>` tree as a ROUTE string: each wire becomes its node,
# and a wire on the same node as the one before it, a join, is no hop at all.
proc node_of {name} {
  global node_cache
  if {![info exists node_cache($name)]} {
    set node_cache($name) [get_nodes -quiet -of_objects [get_wires -quiet $name]]
  }
  return $node_cache($name)
}
# Vivado reads a branch that opens with another branch as leaving the wrong node,
# so the wires are made a tree of nodes first, and written with one brace a child.
proc walk {tree parent} {
  global children
  foreach item $tree {
    # Reticle writes a branch with spaces inside its braces, so one of a single
    # wire is still told from a wire.
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
set routes_short 0
proc reticle_route {net tree} {
  global routes_set routes_refused routes_short
  set inner [route_string $tree]
  # One node is a route that never leaves its site.
  if {[llength [regexp -all -inline {\S+/\S+} $inner]] < 2} { incr routes_short; return }
  set object [get_nets -quiet $net]
  if {$object eq ""} { say no_net $net; incr routes_refused; return }
  if {[catch {set_property FIXED_ROUTE "\{$inner \}" $object} message]} {
    incr routes_refused
    say refused $net [string map {"\n" " "} $message]
    say refused_route $inner
  } else {
    incr routes_set
  }
}

say part $part
say vivado [version -short]
say stage $stage
set finished 0
step link {
  # A project in memory refuses write_bitstream afterwards, as not implemented.
  set_property design_mode GateLvl [current_fileset]
  read_verilog $stem.v
  read_xdc $stem.xdc
  link_design -top $top -part $part
}
if {$stage eq "netlist"} {
  step place { place_design }
} else {
  step place { source $stem.tcl }
}
if {$stage eq "routes"} {
  step set_routes { source $stem.routes.tcl }
  say routes_set $routes_set
  say routes_inside_a_site $routes_short
  say routes_refused $routes_refused
  # What Reticle leaves alone: a tied pin's constant, which is a property of
  # the fabric to it and a net to Vivado.
  # A refusal here is an answer about one net, so it does not end the run: the
  # rest are still Reticle's routes, and still timed. The nets that are not
  # finished are listed below.
  set t0 [clock milliseconds]
  if {[catch { route_design -preserve } message]} {
    say route_rest_refused [string map {"\n" " "} $message]
    set finished 0
  } else {
    set finished 1
  }
  say seconds route_rest [format %.3f [expr {([clock milliseconds] - $t0) / 1000.0}]]
} else {
  step route { route_design }
}
report_route_status -file route_status.rpt
report_timing_summary -file timing_summary.rpt
report_timing -max_paths 10 -nworst 1 -file timing.rpt
report_utilization -file utilization.rpt
foreach {ref n} [apply {{} {
  set counts [dict create]
  foreach c [get_cells -hier -filter {IS_PRIMITIVE}] { dict incr counts [get_property REF_NAME $c] }
  return $counts
}}] { say cell $ref $n }
set unrouted [get_nets -quiet -hier -filter {ROUTE_STATUS != INTRASITE && ROUTE_STATUS != ROUTED && ROUTE_STATUS != NOLOADS && ROUTE_STATUS != NODRIVER}]
say nets_not_routed [llength $unrouted]
foreach net [lrange $unrouted 0 19] { say not_routed $net [get_property ROUTE_STATUS $net] }
foreach clock [get_clocks -quiet] {
  set path [get_timing_paths -group $clock -max_paths 1 -quiet]
  if {[llength $path]} {
    say clock $clock period [get_property PERIOD $clock] slack [get_property SLACK $path] delay [get_property DATAPATH_DELAY $path] levels [get_property LOGIC_LEVELS $path] from [get_property STARTPOINT_PIN $path] to [get_property ENDPOINT_PIN $path]
  }
}
catch { say wns [get_property SLACK [get_timing_paths -max_paths 1 -quiet]] }
# Every endpoint's worst path: this is what compares with Reticle's estimate of
# the same endpoint, since the worst of each may not be the same path.
set list [open $top.endpoints w]
foreach clock [get_clocks -quiet] {
  foreach path [get_timing_paths -quiet -group $clock -max_paths 100000 -nworst 1] {
    puts $list "$clock [get_property ENDPOINT_PIN $path] [get_property STARTPOINT_PIN $path] [get_property DATAPATH_DELAY $path] [get_property SLACK $path] [get_property LOGIC_LEVELS $path]"
  }
}
close $list
# Every connection's delay, slow corner in ps, the other way to ask the same.
set list [open $top.connections w]
foreach net [get_nets -quiet -hier -filter {TYPE == SIGNAL}] {
  foreach delay [get_net_delays -quiet -of_objects $net -interconnect_only] {
    puts $list "[get_property TO_PIN $delay] [get_property SLOW_MAX $delay] [get_property FAST_MAX $delay]"
  }
}
close $list
set pips 0
set list [open $top.pips w]
foreach net [get_nets -quiet -hier -top_net_of_hierarchical_group] {
  set here [get_pips -quiet -of_objects $net]
  incr pips [llength $here]
  foreach pip [lsort $here] { puts $list "$net $pip" }
}
close $list
say routed_pips $pips
# These bitstreams are for decoding and never for a part. Vivado will not write
# one for a design with an unfinished net: RTSTAT-2, -5 and -9 "may not change
# severity", and that is left as its answer. Those designs still have the
# placement stage's bitstream, whose cells are Reticle's and whose routes are not.
# A checkpoint all the same of routes Vivado could not finish, for
# timing_dump.tcl: the nets that are routed are still Reticle's, and timed.
if {!$finished && $stage eq "routes"} { write_checkpoint -force $top.dcp }
if {$finished || $stage eq "placement"} {
  step bitstream {
    # A design placed by place_cell is "not implemented" to write_bitstream;
    # the same design read back from a checkpoint is. A severity does not survive
    # that, so these come after it.
    write_checkpoint -force $top.dcp
    close_design
    open_checkpoint $top.dcp
    set_property SEVERITY {Warning} [get_drc_checks NSTD-1]
    set_property SEVERITY {Warning} [get_drc_checks UCIO-1]
    write_bitstream -force $top.bit
    if {![file exists $top.bit]} { error "write_bitstream wrote no file" }
  }
}
say end
