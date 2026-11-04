# One design through Vivado, with what it did written to <top>.result.
#   vivado -mode batch -source run_vivado.tcl -tclargs <dir> <top> <part> <out>
# Each step is timed by the clock in here, not parsed from a log, and a step
# that fails says which step it was: a probe that Vivado refuses is a fact the
# parity table wants.
lassign $argv dir top part out
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
say part $part
say vivado [version -short]
step read {
  read_verilog $dir/$top.v
  if {[file exists $dir/$top.xdc]} { read_xdc $dir/$top.xdc }
}
step synth { synth_design -top $top -part $part }
foreach {ref n} [apply {{} {
  set counts [dict create]
  foreach c [get_cells -hier -filter {IS_PRIMITIVE}] { dict incr counts [get_property REF_NAME $c] }
  return $counts
}}] { say synth_cell $ref $n }
step opt { opt_design }
step place { place_design }
step route { route_design }
foreach {ref n} [apply {{} {
  set counts [dict create]
  foreach c [get_cells -hier -filter {IS_PRIMITIVE}] { dict incr counts [get_property REF_NAME $c] }
  return $counts
}}] { say cell $ref $n }
foreach {site n} [apply {{} {
  set counts [dict create]
  foreach s [get_sites -quiet -of_objects [get_cells -quiet -hier -filter {IS_PRIMITIVE && REF_NAME !~ GND && REF_NAME !~ VCC}]] { dict incr counts [get_property SITE_TYPE $s] }
  return $counts
}}] { say site $site $n }
foreach clock [get_clocks -quiet] {
  set path [get_timing_paths -group $clock -max_paths 1 -quiet]
  if {[llength $path]} {
    say clock $clock period [get_property PERIOD $clock] slack [get_property SLACK $path] delay [get_property DATAPATH_DELAY $path] levels [get_property LOGIC_LEVELS $path]
  }
}
catch { say wns [get_property SLACK [get_timing_paths -max_paths 1 -quiet]] }
set pips 0
foreach net [get_nets -quiet -hier -top_net_of_hierarchical_group -filter {TYPE != POWER && TYPE != GROUND}] {
  incr pips [llength [get_pips -quiet -of_objects $net]]
}
say routed_pips $pips
# `-quiet` would hide a refusal here, and a table with no `.bit` behind it.
step bitstream {
  write_bitstream -force $top.bit
  if {![file exists $top.bit]} { error "write_bitstream wrote no file" }
}
say end
